use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use selenite_mcp::Server;
use serde_json::{Value, json};

fn vault() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".local/tests/mcp")
        .join(format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("Alpha.md"),
        "# Alpha\n\nA searchable nebula. [[Beta]]\n",
    )
    .unwrap();
    fs::write(root.join("Beta.md"), "# Beta\n").unwrap();
    root
}

fn initialize(server: &mut Server) {
    let response = server.handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"tests","version":"1"}}})).unwrap();
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
    assert!(
        server
            .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .is_none()
    );
}

fn call(server: &mut Server, name: &str, arguments: Value) -> Value {
    server.handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":arguments}})).unwrap()
}

#[test]
fn handshake_search_and_path_containment() {
    let mut server = Server::new(vault(), false).unwrap();
    assert_eq!(
        call(&mut server, "vault_list_pages", json!({}))["error"]["code"],
        -32002
    );
    initialize(&mut server);
    let results = call(&mut server, "vault_search", json!({"query":"nebula"}));
    assert_eq!(
        results["result"]["structuredContent"]["results"][0]["path"],
        "Alpha.md"
    );
    let backlinks = call(&mut server, "vault_backlinks", json!({"path":"Beta.md"}));
    assert_eq!(
        backlinks["result"]["structuredContent"]["backlinks"][0]["path"],
        "Alpha.md"
    );
    assert_eq!(
        call(
            &mut server,
            "vault_read_page",
            json!({"path":"../outside.md"})
        )["result"]["isError"],
        true
    );
    assert_eq!(
        call(
            &mut server,
            "vault_create_page",
            json!({"path":"New.md","source":"x"})
        )["error"]["code"],
        -32602
    );
}

#[test]
fn writes_are_explicit_and_compare_source() {
    let root = vault();
    let mut server = Server::new(&root, true).unwrap();
    initialize(&mut server);
    assert_eq!(
        call(
            &mut server,
            "vault_write_page",
            json!({"path":"Beta.md","expectedSource":"stale","source":"replacement"})
        )["result"]["isError"],
        true
    );
    assert_eq!(
        fs::read_to_string(root.join("Beta.md")).unwrap(),
        "# Beta\n"
    );
    assert_eq!(
        call(
            &mut server,
            "vault_write_page",
            json!({"path":"Beta.md","expectedSource":"# Beta\n","source":"# Saved\n"})
        )["result"]["isError"],
        false
    );
    assert_eq!(
        fs::read_to_string(root.join("Beta.md")).unwrap(),
        "# Saved\n"
    );
}

#[test]
fn stdio_recovers_from_bad_json_and_has_only_newline_json() {
    let mut server = Server::new(vault(), false).unwrap();
    let mut output = Vec::new();
    server
        .serve(
            Cursor::new(b"invalid\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n"),
            &mut output,
        )
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    let messages: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["error"]["code"], -32700);
    assert_eq!(messages[1]["id"], 7);
    assert!(text.ends_with('\n'));
}
