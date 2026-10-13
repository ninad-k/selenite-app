# Plugin hosting and configurable catalogs

`selenite-plugin-host` embeds Wasmtime to execute core WebAssembly modules. Plugins are disabled by default. An enabled configuration, an explicit run, a manifest capability request and a matching per-plugin grant are all needed for vault access. Installing a package grants no capabilities and does not execute it.

## Configuration

The desktop service settings store this TOML configuration in `.selenite/plugins.toml` through the core's compare-and-swap metadata API. The CLI reads the same file. The public `PluginConfig` type validates it; no built-in catalog endpoint or trusted publisher key is supplied.

```toml
enabled = false
# catalog_url = "https://your-publisher.example/catalog.json"

[trusted_catalog_keys]
# publisher_key_id = "base64-encoded-32-byte-Ed25519-public-key"

[grants]
# list-pages = ["vault_list"]
# explicitly-reviewed-writer = ["vault_read", "vault_write"]

[limits]
fuel = 5000000
memory_bytes = 16777216
max_module_bytes = 2097152
max_message_bytes = 1048576
max_host_calls = 64
```

The available grants are `vault_list`, `vault_read` and `vault_write`. The runtime intersects grants with the manifest's requests. A manifest cannot grant itself access, and a configured grant cannot enable an undeclared capability. The host supplies no WASI, HTTP, shell, environment or general filesystem imports. Read and write host calls validate vault-relative Markdown paths; writes use exact expected bytes or exclusive creation, with the core's retained history.

Wasmtime fuel, stack, linear-memory, instance, memory, table and table-element limits constrain guests. Host requests and responses have byte limits; note reads use bounded core reads; enumeration has entry and accumulated-path-byte caps. HTTP responses and installed package reads are bounded before decoding. ABI ranges are checked before dispatch. These are application-level sandbox controls, not a claim of an operating-system-wide CPU or memory quota: JIT compilation and host filesystem work are outside guest fuel accounting. SDK limits and error behavior are specified in [the ABI guide](../sdk/README.md#abi).

## Signed catalog and package installation

Catalog access occurs only on explicit fetch/install calls. Endpoints require HTTPS; HTTP is accepted only for loopback development servers. Requests have timeouts, do not follow redirects, and enforce body size limits. A hosted marketplace, payment system or publisher service is not deployed by this implementation.

The endpoint returns this envelope:

```json
{
  "key_id": "publisher_key_id",
  "payload": "{\"issued_at\":1791849600,\"expires_at\":1791936000,\"entries\":[]}",
  "signature": "base64-encoded-64-byte-Ed25519-signature"
}
```

The signature covers the UTF-8 bytes `selenite.catalog.v1\n` followed by the **exact UTF-8 payload string bytes**. The sequence `\n` here means one newline byte. Publishers serialize their payload once, sign those exact bytes and retain the same string in the envelope. Verification uses the configured public key and Ed25519 strict verification, never a key delivered by the catalog itself.

Decoded payload shape:

```json
{
  "issued_at": 1791849600,
  "expires_at": 1791936000,
  "entries": [{
    "manifest": {
      "id": "echo",
      "name": "JSON echo",
      "version": "1.0.0",
      "abi_version": 1,
      "capabilities": []
    },
    "url": "https://your-publisher.example/echo.wasm",
    "sha256": "64-lowercase-hex-digits-of-the-exact-wasm-bytes"
  }]
}
```

Times are Unix seconds. Fetch and installation require `issued_at <= now < expires_at`. Catalog IDs must be unique. Signatures cover the manifest, requested capabilities, download URL and expected SHA-256. Downloads are installed only when their bytes match that digest; a signed document is reverified at installation under current trusted-key settings.

Packages are JSON/base64 files under `.selenite/plugins/packages/<id>/<package-sha256>.json`. Their content address covers the manifest, module and signed catalog envelope. Active records live under `.selenite/plugins/installed/<id>.json`. The core safely reads and writes these metadata paths. Installation writes immutable package content first, then publishes the active record with compare-and-swap. Prior packages and metadata history are retained; there is no deletion or automatic cleanup.

Every installed run checks record/package consistency, content hashes and the original catalog signature under the current trusted keys. An expired catalog prevents a new installation but does not disable an already installed package; removing its trusted key prevents subsequent runs. The installer provides integrity and authenticity under configured keys, not an audit of a publisher's behavior. Existing grants remain explicit settings when updating the same plugin ID; newly requested capabilities remain denied until separately granted.

## Public API and verification

- `PluginConfig::validate`, `PluginHost::new`, and `PluginHost::run` configure and execute explicitly supplied modules.
- `CatalogClient::fetch_catalog`, `verify_catalog`, `CatalogClient::install`, and `install_from_bytes` verify and install configured or deliberately selected packages.
- `list_installed`, `load_installed`, and `run_installed` inspect stored packages and execute them under current settings. `load_installed` checks stored integrity; `run_installed` additionally verifies publisher trust and applies grants.

Tests run original Wasm modules and cover fuel exhaustion, memory growth, denied imports, invalid pointers, denied capabilities, bounded reads/call counts, compare-and-swap conflicts, a local HTTP catalog/package round trip, tampered signatures/packages, retained versions and revoked keys. Original WAT and Rust examples are in [the SDK](../sdk/README.md).

## Dependency choices

| Direct dependency | Version selected | License | Purpose |
| --- | --- | --- | --- |
| `wasmtime` | `=36.0.17` | Apache-2.0 WITH LLVM-exception | Execute WebAssembly with explicit imports, fuel and memory limits; the pinned LTS patch declares Rust 1.86 compatibility. |
| `ed25519-dalek` | `2.2.0` | BSD-3-Clause | Verify catalog and entitlement signatures with strict Ed25519 checks. |
| `sha2` | `0.10.9` | MIT OR Apache-2.0 | Verify exact module bytes and address retained package content. |
| `base64` | `0.22.1` | MIT OR Apache-2.0 | Encode keys, signatures and bounded module bytes in JSON metadata. |
| `reqwest` | `0.13.5` | MIT OR Apache-2.0 | Fetch explicitly configured catalogs and packages with TLS, timeouts and bounded reads. |
| `url` | `2.5.8` | MIT OR Apache-2.0 | Validate service schemes, hosts, loopback exceptions and embedded credentials. |
| `serde` / `serde_json` | `1.0.229` / `1.0.154` | MIT OR Apache-2.0 | Define strict configuration/manifest/ABI documents and serialize them. |
| `anyhow` | `1.0.104` | MIT OR Apache-2.0 | Preserve contextual runtime, integrity and transport errors. |
| `wat` (development) | `=1.236.1` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compile original WAT fixtures and SDK examples for real execution tests. |

Wasmtime uses only the `cranelift`, `runtime` and `std` features; no WASI crate is added. Version 36 is an LTS release under the upstream [support policy](https://docs.wasmtime.dev/stability-release.html), and [36.0.17](https://github.com/bytecodealliance/wasmtime/releases/tag/v36.0.17) was selected after checking current patches. No workspace Rust-version change is needed. The new dependency graph was checked before compilation; it adds no non-permissive license exception.
