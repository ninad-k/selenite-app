use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

fn fixture() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".local/tests/cli-services")
        .join(format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Readme.md"), "# Hello\n\nAuthoritative bytes.\n").unwrap();
    root
}
fn cli(command: &str, root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_selenite"))
        .args([command])
        .arg(root)
        .output()
        .unwrap()
}

#[test]
fn optional_services_stay_disabled_and_templates_are_plain_files() {
    let root = fixture();
    for command in ["sync-plan", "catalog", "ai-index"] {
        let result = cli(command, &root);
        assert!(!result.status.success(), "{command}");
    }
    let result = cli("plugins", &root);
    assert!(result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap(),
        json!([])
    );
    let result = cli("pro-status", &root);
    assert!(result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["active"],
        false
    );
    let result = cli("init-gtd", &root);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(root.join(".selenite/schemas/gtd.toml").exists());
    let result = cli("views", &root);
    assert!(result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert_eq!(
        fs::read_to_string(root.join("Readme.md")).unwrap(),
        "# Hello\n\nAuthoritative bytes.\n"
    );
}

#[test]
fn actual_mcp_process_has_clean_stdout_and_reads_markdown() {
    let root = fixture();
    let mut child = Command::new(env!("CARGO_BIN_EXE_selenite"))
        .arg("mcp")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"vault_read_page","arguments":{"path":"Readme.md"}}}),
    ];
    let mut input = child.stdin.take().unwrap();
    for request in requests {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let messages: Vec<Value> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[1]["result"]["structuredContent"]["source"],
        "# Hello\n\nAuthoritative bytes.\n"
    );
}
