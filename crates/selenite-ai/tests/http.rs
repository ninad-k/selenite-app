use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread,
};

use selenite_ai::{AiConfig, ChatMessage, EmbeddingProvider, HttpProvider, LlmProvider};
use serde_json::{Value, json};

#[test]
fn compatible_http_transport_sends_models_and_reorders_embeddings() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        for expected in ["embeddings", "chat/completions"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            assert!(first.starts_with(&format!("POST /v1/{expected} ")));
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            let response = if expected == "embeddings" {
                assert_eq!(body["model"], "embedding-test");
                assert_eq!(body["input"].as_array().unwrap().len(), 2);
                json!({"data":[{"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]})
            } else {
                assert_eq!(body["model"], "chat-test");
                json!({"choices":[{"message":{"content":"Grounded [S1]."}}]})
            };
            let text = response.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",text.len(),text).unwrap();
        }
    });
    let provider = HttpProvider::new(AiConfig {
        enabled: true,
        base_url,
        chat_model: "chat-test".to_owned(),
        embedding_model: "embedding-test".to_owned(),
        api_key_env: None,
        max_context_chars: 12000,
        folders: Vec::new(),
    })
    .unwrap();
    assert_eq!(
        provider
            .embed(&["one".to_owned(), "two".to_owned()])
            .unwrap(),
        vec![vec![1.0, 0.0], vec![0.0, 1.0]]
    );
    assert_eq!(
        provider
            .complete(&[ChatMessage {
                role: "user".to_owned(),
                content: "question".to_owned()
            }])
            .unwrap(),
        "Grounded [S1]."
    );
    worker.join().unwrap();
}
