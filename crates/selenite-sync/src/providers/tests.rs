use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Instant,
};

struct Reply {
    status: u16,
    body: String,
    location: Option<String>,
}
fn json_reply(value: Value) -> Reply {
    Reply {
        status: 200,
        body: value.to_string(),
        location: None,
    }
}
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    handle: JoinHandle<()>,
}
impl Server {
    fn start(replies: impl FnOnce(&str) -> Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let replies = replies(&base);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let handle = thread::spawn(move || {
            for reply in replies {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => panic!("mock accept failed: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(header_end) =
                        request.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        let header = String::from_utf8_lossy(&request[..header_end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= header_end + 4 + length {
                            break;
                        }
                    }
                }
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                let location = reply
                    .location
                    .map(|url| format!("Location: {url}\r\n"))
                    .unwrap_or_default();
                write!(stream, "HTTP/1.1 {} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{location}Connection: close\r\n\r\n{}", reply.status, reply.body.len(), reply.body).unwrap();
            }
        });
        Self {
            base,
            requests,
            handle,
        }
    }
    fn finish(self) -> Vec<String> {
        self.handle.join().unwrap();
        self.requests.lock().unwrap().clone()
    }
}

// Construct private transport directly: tests never mutate process environment,
// contact a real account, or use a real credential.
fn http() -> Http {
    Http {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        token: "test-token".into(),
    }
}
fn config(base: &str, provider: ProviderKind) -> SyncConfig {
    SyncConfig {
        enabled: true,
        provider,
        endpoint: base.into(),
        folder_id: "folder".into(),
        ..SyncConfig::default()
    }
}
fn drive(base: &str) -> GoogleDrive {
    GoogleDrive {
        config: config(base, ProviderKind::GoogleDrive),
        http: http(),
        base: endpoint(base).unwrap(),
        upload: endpoint(base).unwrap(),
    }
}
fn graph(base: &str) -> OneDrive {
    OneDrive {
        config: config(base, ProviderKind::OneDrive),
        http: http(),
        base: endpoint(base).unwrap(),
    }
}
fn drive_file(version: &RemoteVersion) -> Value {
    json!({"id":"file","name":archive_name(&version.revision),"parents":["folder"],"appProperties":{"seleniteFormat":"1"}})
}
fn graph_file(version: &RemoteVersion) -> Value {
    json!({"id":"file","name":archive_name(&version.revision),"file":{},"parentReference":{"id":"folder"}})
}

#[test]
fn drive_paginates_and_reads_verified_media() {
    let version = RemoteVersion::new("Note.md", vec![], "source").unwrap();
    let server = Server::start(|_| {
        vec![
            json_reply(json!({"files":[],"nextPageToken":"next"})),
            json_reply(json!({"files":[drive_file(&version)]})),
            json_reply(serde_json::to_value(&version).unwrap()),
        ]
    });
    assert_eq!(drive(&server.base).versions().unwrap(), vec![version]);
    let requests = server.finish();
    assert!(requests[0].contains("parents"));
    assert!(requests[1].contains("pageToken=next"));
    assert!(requests[2].contains("alt=media"));
}

#[test]
fn drive_upload_always_creates_multipart_immutable_object() {
    let version = RemoteVersion::new("Note.md", vec![], "source").unwrap();
    let server = Server::start(|_| vec![json_reply(json!({"id":"new-id"}))]);
    drive(&server.base).publish(&version).unwrap();
    let requests = server.finish();
    assert!(requests[0].starts_with("POST /files?uploadType=multipart"));
    assert!(requests[0].contains("multipart/related"));
    assert!(requests[0].contains(&version.revision));
    assert!(!requests[0].starts_with("PATCH"));
}

#[test]
fn graph_pagination_and_download_redirect_do_not_forward_bearer() {
    let version = RemoteVersion::new("Note.md", vec![], "source").unwrap();
    let server = Server::start(|base| {
        vec![
            json_reply(
                json!({"value":[],"@odata.nextLink":format!("{base}me/drive/items/folder/children?page=2")}),
            ),
            json_reply(json!({"value":[graph_file(&version)]})),
            Reply {
                status: 302,
                body: String::new(),
                location: Some(format!("{base}download")),
            },
            json_reply(serde_json::to_value(&version).unwrap()),
        ]
    });
    assert_eq!(graph(&server.base).versions().unwrap(), vec![version]);
    let requests = server.finish();
    assert!(
        requests[2]
            .to_lowercase()
            .contains("authorization: bearer test-token")
    );
    assert!(!requests[3].to_lowercase().contains("authorization:"));
}

#[test]
fn graph_upload_requires_fail_on_conflict_and_unauthenticated_upload_url() {
    let version = RemoteVersion::new("Note.md", vec![], "source").unwrap();
    let server = Server::start(|base| {
        vec![
            json_reply(json!({"uploadUrl":format!("{base}upload?signature=private")})),
            Reply {
                status: 201,
                body: "{}".into(),
                location: None,
            },
        ]
    });
    graph(&server.base).publish(&version).unwrap();
    let requests = server.finish();
    assert!(requests[0].contains("createUploadSession"));
    assert!(requests[0].contains("\"@microsoft.graph.conflictBehavior\":\"fail\""));
    assert!(requests[1].starts_with("PUT /upload?signature=private"));
    assert!(
        requests[1]
            .to_lowercase()
            .contains("content-range: bytes 0-")
    );
    assert!(!requests[1].to_lowercase().contains("authorization:"));
}

#[test]
fn graph_create_race_is_idempotent_only_for_identical_existing_bytes() {
    let version = RemoteVersion::new("Note.md", vec![], "source").unwrap();
    let server = Server::start(|_| {
        vec![
            Reply {
                status: 409,
                body: "{}".into(),
                location: None,
            },
            json_reply(json!({"value":[graph_file(&version)]})),
            json_reply(serde_json::to_value(&version).unwrap()),
        ]
    });
    graph(&server.base).publish(&version).unwrap();
    assert_eq!(server.finish().len(), 3);
}

#[test]
fn errors_and_untrusted_pagination_stop_without_disclosing_response_secrets() {
    let server = Server::start(|_| {
        vec![
            Reply {
                status: 401,
                body: "private-token".into(),
                location: None,
            },
            Reply {
                status: 404,
                body: "private-token".into(),
                location: None,
            },
            json_reply(json!({"value":[],"@odata.nextLink":"https://untrusted.invalid/stolen"})),
        ]
    });
    let drive = drive(&server.base);
    let unauthorized = drive.versions().unwrap_err().to_string();
    assert!(unauthorized.contains("401") && !unauthorized.contains("private-token"));
    assert!(drive.versions().unwrap_err().to_string().contains("404"));
    assert!(
        graph(&server.base)
            .versions()
            .unwrap_err()
            .to_string()
            .contains("pagination escaped")
    );
    assert_eq!(server.finish().len(), 3);
}
