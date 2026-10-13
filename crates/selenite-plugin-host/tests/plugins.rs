use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use selenite_core::vault::Vault;
use selenite_plugin_host::{
    CATALOG_SIGNING_DOMAIN, Capability, Catalog, CatalogClient, CatalogEntry, PluginConfig,
    PluginHost, PluginManifest, SignedDocument, list_installed, load_installed,
    pro::{ENTITLEMENT_SIGNING_DOMAIN, Entitlement, ProConfig, verify_entitlement},
    run_installed, verify_catalog,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = repository.join(".local").join("tests").join(format!(
        "plugin-{}-{}-{}",
        std::process::id(),
        now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn manifest(capabilities: &[Capability]) -> PluginManifest {
    PluginManifest {
        id: "test-plugin".into(),
        name: "Test plugin".into(),
        version: "1.0.0".into(),
        abi_version: 1,
        capabilities: capabilities.iter().copied().collect(),
    }
}

fn config() -> PluginConfig {
    PluginConfig {
        enabled: true,
        ..PluginConfig::default()
    }
}

fn echo() -> Vec<u8> {
    wat::parse_str(
        r#"(module
      (memory (export "memory") 1 256)
      (func (export "alloc") (param i32) (result i32) (i32.const 4096))
      (func (export "run") (param $p i32) (param $n i32) (result i64)
        (i64.or (i64.shl (i64.extend_i32_u (local.get $p)) (i64.const 32))
                (i64.extend_i32_u (local.get $n)))))"#,
    )
    .unwrap()
}

fn call_module(request: Value, calls: u32) -> Vec<u8> {
    let request = serde_json::to_vec(&request).unwrap();
    let data = request
        .iter()
        .map(|byte| format!("\\{byte:02x}"))
        .collect::<String>();
    let repeat = (1..calls).map(|_| format!("(drop (call $host (i32.const 0) (i32.const {}) (i32.const 65536) (i32.const 1048576)))", request.len())).collect::<String>();
    wat::parse_str(format!(r#"(module
      (import "selenite" "call" (func $host (param i32 i32 i32 i32) (result i32)))
      (memory (export "memory") 32 256)
      (data (i32.const 0) "{data}")
      (func (export "alloc") (param i32) (result i32) (i32.const 4096))
      (func (export "run") (param i32 i32) (result i64)
        {repeat}
        (i64.or (i64.shl (i64.const 65536) (i64.const 32))
          (i64.extend_i32_u (call $host (i32.const 0) (i32.const {}) (i32.const 65536) (i32.const 1048576))))))"#, request.len())).unwrap()
}

fn execute(
    wasm: &[u8],
    capabilities: &[Capability],
    granted: &[Capability],
    vault: &Vault,
) -> anyhow::Result<selenite_plugin_host::PluginRun> {
    PluginHost::new(&config())?.run(
        vault,
        &manifest(capabilities),
        wasm,
        &granted.iter().copied().collect(),
        json!({"hello":"vault"}),
    )
}

#[test]
fn executes_real_wasm_and_starts_disabled() {
    let vault = Vault::open(scratch()).unwrap();
    assert!(PluginHost::new(&PluginConfig::default()).is_err());
    assert!(CatalogClient::new(&PluginConfig::default()).is_err());
    let result = execute(&echo(), &[], &[], &vault).unwrap();
    assert_eq!(result.output, json!({"hello":"vault"}));
    assert!(result.fuel_consumed > 0);
    assert_eq!(result.host_calls, 0);
}

#[test]
fn bundled_wat_examples_execute_the_documented_abi() {
    let vault = Vault::open(scratch()).unwrap();
    vault.create_page("Example.md", "# SDK fixture").unwrap();
    let echo = wat::parse_str(include_str!("../../../sdk/wat/echo.wat")).unwrap();
    let echo_manifest: PluginManifest =
        serde_json::from_str(include_str!("../../../sdk/wat/echo.manifest.json")).unwrap();
    let host = PluginHost::new(&config()).unwrap();
    assert_eq!(
        host.run(
            &vault,
            &echo_manifest,
            &echo,
            &BTreeSet::new(),
            json!({"sdk":true})
        )
        .unwrap()
        .output,
        json!({"sdk":true})
    );
    let list = wat::parse_str(include_str!("../../../sdk/wat/list-pages.wat")).unwrap();
    let list_manifest: PluginManifest =
        serde_json::from_str(include_str!("../../../sdk/wat/list-pages.manifest.json")).unwrap();
    let result = host
        .run(
            &vault,
            &list_manifest,
            &list,
            &BTreeSet::from([Capability::VaultList]),
            json!(null),
        )
        .unwrap();
    assert_eq!(result.output["data"]["paths"], json!(["Example.md"]));
}

#[test]
fn fuel_and_memory_limits_stop_guests() {
    let vault = Vault::open(scratch()).unwrap();
    let looping = wat::parse_str(r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "run") (param i32 i32) (result i64) (loop $forever (br $forever)) (i64.const 0)))"#).unwrap();
    let mut limited = config();
    limited.limits.fuel = 1000;
    let error = PluginHost::new(&limited)
        .unwrap()
        .run(
            &vault,
            &manifest(&[]),
            &looping,
            &BTreeSet::new(),
            json!(null),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("fuel"));
    let growing = wat::parse_str(r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "run") (param i32 i32) (result i64) (drop (memory.grow (i32.const 1000))) (i64.const 0)))"#).unwrap();
    let error = execute(&growing, &[], &[], &vault).unwrap_err();
    assert!(format!("{error:#}").contains("memory"));
    let initial = wat::parse_str(r#"(module (memory (export "memory") 1000))"#).unwrap();
    assert!(execute(&initial, &[], &[], &vault).is_err());
}

#[test]
fn rejects_wasi_unknown_imports_and_invalid_memory_ranges() {
    let vault = Vault::open(scratch()).unwrap();
    let wasi = wat::parse_str(r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))))"#).unwrap();
    let error = execute(&wasi, &[], &[], &vault).unwrap_err();
    assert!(format!("{error:#}").contains("unsupported import"));
    let invalid = wat::parse_str(
        r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "run") (param i32 i32) (result i64)
        (i64.or (i64.shl (i64.const 4294967290) (i64.const 32)) (i64.const 100))))"#,
    )
    .unwrap();
    assert!(execute(&invalid, &[], &[], &vault).is_err());
}

#[test]
fn requested_and_granted_capabilities_are_both_required() {
    let root = scratch();
    let vault = Vault::open(&root).unwrap();
    vault.create_page("Note.md", "Original Markdown.").unwrap();
    let wasm = call_module(json!({"op":"read","path":"Note.md"}), 1);
    for (requested, granted) in [
        (&[Capability::VaultRead][..], &[][..]),
        (&[][..], &[Capability::VaultRead][..]),
    ] {
        let result = execute(&wasm, requested, granted, &vault).unwrap();
        assert_eq!(result.output["ok"], false);
        assert!(
            result.output["error"]
                .as_str()
                .unwrap()
                .contains("not granted")
        );
    }
    let result = execute(
        &wasm,
        &[Capability::VaultRead],
        &[Capability::VaultRead],
        &vault,
    )
    .unwrap();
    assert_eq!(result.output["data"]["source"], "Original Markdown.");
    assert_eq!(result.host_calls, 1);
    let list = execute(
        &call_module(json!({"op":"list","limit":10}), 1),
        &[Capability::VaultList],
        &[Capability::VaultList],
        &vault,
    )
    .unwrap();
    assert_eq!(list.output["data"]["paths"], json!(["Note.md"]));
    for path in ["../outside.md", ".selenite/secret.md", "C:/secret.md"] {
        let result = execute(
            &call_module(json!({"op":"read","path":path}), 1),
            &[Capability::VaultRead],
            &[Capability::VaultRead],
            &vault,
        )
        .unwrap();
        assert_eq!(result.output["ok"], false);
    }
}

#[test]
fn writes_require_permission_and_exact_source_without_lost_updates() {
    let vault = Vault::open(scratch()).unwrap();
    vault.create_page("Note.md", "before").unwrap();
    let wasm = call_module(
        json!({"op":"write","path":"Note.md","expected_source":"before","source":"after"}),
        1,
    );
    let denied = execute(&wasm, &[Capability::VaultWrite], &[], &vault).unwrap();
    assert_eq!(denied.output["ok"], false);
    assert_eq!(vault.read_source("Note.md").unwrap(), "before");
    let changed = execute(
        &wasm,
        &[Capability::VaultWrite],
        &[Capability::VaultWrite],
        &vault,
    )
    .unwrap();
    assert_eq!(changed.output["ok"], true);
    assert_eq!(vault.read_source("Note.md").unwrap(), "after");
    assert_eq!(
        execute(
            &wasm,
            &[Capability::VaultWrite],
            &[Capability::VaultWrite],
            &vault
        )
        .unwrap()
        .output["ok"],
        false
    );
    let overwrite = call_module(
        json!({"op":"write","path":"Note.md","expected_source":null,"source":"overwrite"}),
        1,
    );
    assert_eq!(
        execute(
            &overwrite,
            &[Capability::VaultWrite],
            &[Capability::VaultWrite],
            &vault
        )
        .unwrap()
        .output["ok"],
        false
    );
    assert_eq!(vault.read_source("Note.md").unwrap(), "after");
    let create = call_module(
        json!({"op":"write","path":"New.md","expected_source":null,"source":"# New"}),
        1,
    );
    assert_eq!(
        execute(
            &create,
            &[Capability::VaultWrite],
            &[Capability::VaultWrite],
            &vault
        )
        .unwrap()
        .output["ok"],
        true
    );
    assert_eq!(vault.read_source("New.md").unwrap(), "# New");
}

#[test]
fn host_call_count_and_source_size_are_bounded() {
    let vault = Vault::open(scratch()).unwrap();
    let mut limited = config();
    limited.limits.max_host_calls = 1;
    let wasm = call_module(json!({"op":"list"}), 2);
    let result = PluginHost::new(&limited).unwrap().run(
        &vault,
        &manifest(&[Capability::VaultList]),
        &wasm,
        &BTreeSet::from([Capability::VaultList]),
        json!(null),
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("host-call limit"));
    vault.create_page("Large.md", &"x".repeat(200_000)).unwrap();
    let result = execute(
        &call_module(json!({"op":"read","path":"Large.md"}), 1),
        &[Capability::VaultRead],
        &[Capability::VaultRead],
        &vault,
    )
    .unwrap();
    assert_eq!(result.output["ok"], false);
}

fn signed(payload: String, domain: &[u8], key: &SigningKey) -> SignedDocument {
    let mut message = domain.to_vec();
    message.extend_from_slice(payload.as_bytes());
    SignedDocument {
        key_id: "test-issuer".into(),
        payload,
        signature: STANDARD.encode(key.sign(&message).to_bytes()),
    }
}

fn catalog_config(key: &SigningKey) -> PluginConfig {
    let mut config = config();
    config.trusted_catalog_keys.insert(
        "test-issuer".into(),
        STANDARD.encode(key.verifying_key().to_bytes()),
    );
    config
}

fn catalog_document(wasm: &[u8], key: &SigningKey, url: &str, version: &str) -> SignedDocument {
    let mut manifest = manifest(&[]);
    manifest.version = version.into();
    let catalog = Catalog {
        issued_at: now() - 1,
        expires_at: now() + 3600,
        entries: vec![CatalogEntry {
            manifest,
            url: url.into(),
            sha256: format!("{:x}", Sha256::digest(wasm)),
        }],
    };
    signed(
        serde_json::to_string(&catalog).unwrap(),
        CATALOG_SIGNING_DOMAIN,
        key,
    )
}

#[test]
fn signed_catalog_rejects_tampering_and_installs_without_granting_capabilities() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let config = catalog_config(&key);
    let wasm = echo();
    let document = catalog_document(&wasm, &key, "https://example.com/plugin.wasm", "1.0.0");
    let mut tampered = document.clone();
    tampered.payload = tampered.payload.replace("1.0.0", "9.0.0");
    assert!(verify_catalog(&config, &tampered, now()).is_err());
    assert!(verify_catalog(&config, &document, now() + 7200).is_err());
    let verified = verify_catalog(&config, &document, now()).unwrap();
    let vault = Vault::open(scratch()).unwrap();
    let client = CatalogClient::new(&config).unwrap();
    let mut corrupt = wasm.clone();
    corrupt.push(0);
    assert!(
        client
            .install_from_bytes(&vault, &verified, "test-plugin", &corrupt)
            .is_err()
    );
    assert!(list_installed(&vault).unwrap().is_empty());
    let installed = client
        .install_from_bytes(&vault, &verified, "test-plugin", &wasm)
        .unwrap();
    assert!(config.grants.is_empty());
    assert_eq!(list_installed(&vault).unwrap(), vec![installed.clone()]);
    assert_eq!(
        load_installed(&vault, "test-plugin", &config.limits)
            .unwrap()
            .2,
        wasm
    );
    assert_eq!(
        run_installed(&vault, &config, "test-plugin", json!({"portable":true}))
            .unwrap()
            .output,
        json!({"portable":true})
    );
    let newer = catalog_document(&wasm, &key, "https://example.com/plugin.wasm", "2.0.0");
    client
        .install_from_bytes(
            &vault,
            &verify_catalog(&config, &newer, now()).unwrap(),
            "test-plugin",
            &wasm,
        )
        .unwrap();
    assert_eq!(list_installed(&vault).unwrap()[0].version, "2.0.0");
    assert_eq!(
        vault
            .metadata_paths("plugins/packages/test-plugin")
            .unwrap()
            .len(),
        2
    );
    let mut revoked = config.clone();
    revoked.trusted_catalog_keys.clear();
    assert!(run_installed(&vault, &revoked, "test-plugin", json!(null)).is_err());
}

#[test]
fn local_package_and_pointer_tampering_cannot_bypass_catalog_authorization() {
    let key = SigningKey::from_bytes(&[8; 32]);
    let config = catalog_config(&key);
    let wasm = echo();
    let document = catalog_document(&wasm, &key, "https://example.com/plugin.wasm", "1.0.0");
    let vault = Vault::open(scratch()).unwrap();
    let client = CatalogClient::new(&config).unwrap();
    client
        .install_from_bytes(
            &vault,
            &verify_catalog(&config, &document, now()).unwrap(),
            "test-plugin",
            &wasm,
        )
        .unwrap();
    let record_path = Path::new("plugins")
        .join("installed")
        .join("test-plugin.json");
    let record_source = vault.read_metadata(&record_path).unwrap().unwrap();
    let mut record: Value = serde_json::from_str(&record_source).unwrap();
    let package_path = Path::new("plugins")
        .join("packages")
        .join("test-plugin")
        .join(format!(
            "{}.json",
            record["package_sha256"].as_str().unwrap()
        ));
    let package_source = vault.read_metadata(&package_path).unwrap().unwrap();
    let mut package: Value = serde_json::from_str(&package_source).unwrap();
    package["manifest"]["capabilities"] = json!(["vault_write"]);
    let forged = serde_json::to_string(&package).unwrap();
    let forged_digest = format!("{:x}", Sha256::digest(forged.as_bytes()));
    vault
        .write_metadata(
            Path::new("plugins")
                .join("packages")
                .join("test-plugin")
                .join(format!("{forged_digest}.json")),
            None,
            &forged,
        )
        .unwrap();
    record["package_sha256"] = json!(forged_digest);
    record["plugin"]["capabilities"] = json!(["vault_write"]);
    vault
        .write_metadata(
            &record_path,
            Some(&record_source),
            &serde_json::to_string(&record).unwrap(),
        )
        .unwrap();
    assert!(run_installed(&vault, &config, "test-plugin", json!(null)).is_err());
}

#[test]
fn catalog_client_fetches_and_installs_verified_bytes_from_configured_http_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let key = SigningKey::from_bytes(&[9; 32]);
    let wasm = echo();
    let document = catalog_document(&wasm, &key, &format!("{base}/plugin.wasm"), "1.0.0");
    let envelope = serde_json::to_vec(&document).unwrap();
    let served = wasm.clone();
    let worker = thread::spawn(move || {
        for _ in 0..2 {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(value) => break value,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("HTTP mock accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 1024];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    break;
                }
            }
            let body = if request.starts_with(b"GET /catalog.json ") {
                &envelope
            } else {
                assert!(request.starts_with(b"GET /plugin.wasm "));
                &served
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        }
    });
    let mut config = catalog_config(&key);
    config.catalog_url = Some(format!("{base}/catalog.json"));
    let client = CatalogClient::new(&config).unwrap();
    let catalog = client.fetch_catalog(now()).unwrap();
    let vault = Vault::open(scratch()).unwrap();
    client.install(&vault, &catalog, "test-plugin").unwrap();
    worker.join().unwrap();
    assert_eq!(
        load_installed(&vault, "test-plugin", &config.limits)
            .unwrap()
            .2,
        wasm
    );
}

#[test]
fn entitlements_require_valid_signature_time_issuer_and_audience() {
    let key = SigningKey::from_bytes(&[10; 32]);
    let config = ProConfig {
        trusted_keys: BTreeMap::from([(
            "test-issuer".into(),
            STANDARD.encode(key.verifying_key().to_bytes()),
        )]),
        issuer: Some("publisher".into()),
        audience: Some("selenite".into()),
    };
    let entitlement = Entitlement {
        subject: "local-account".into(),
        issuer: "publisher".into(),
        audience: "selenite".into(),
        issued_at: 100,
        expires_at: 200,
        capabilities: BTreeSet::from(["deployment.example_capability".into()]),
    };
    let document = signed(
        serde_json::to_string(&entitlement).unwrap(),
        ENTITLEMENT_SIGNING_DOMAIN,
        &key,
    );
    let verified = verify_entitlement(&config, &document, 150).unwrap();
    assert!(verified.has_capability("deployment.example_capability", 150));
    assert!(!verified.has_capability("deployment.example_capability", 200));
    assert!(verify_entitlement(&config, &document, 99).is_err());
    assert!(verify_entitlement(&config, &document, 200).is_err());
    let mut wrong = config.clone();
    wrong.issuer = Some("someone-else".into());
    assert!(verify_entitlement(&wrong, &document, 150).is_err());
    wrong = config.clone();
    wrong.audience = Some("other-product".into());
    assert!(verify_entitlement(&wrong, &document, 150).is_err());
    let mut tampered = document.clone();
    tampered.payload = tampered.payload.replace("local-account", "forged-account");
    assert!(verify_entitlement(&config, &tampered, 150).is_err());
    let wrong_domain = signed(document.payload, CATALOG_SIGNING_DOMAIN, &key);
    assert!(verify_entitlement(&config, &wrong_domain, 150).is_err());
    // Pro configuration has no dependency on or control over community execution.
    let vault = Vault::open(scratch()).unwrap();
    vault
        .create_page("Community.md", "Still available.")
        .unwrap();
    assert_eq!(
        vault.read_source("Community.md").unwrap(),
        "Still available."
    );
}
