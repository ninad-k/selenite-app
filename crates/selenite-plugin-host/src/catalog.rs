use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::blocking::Client;
use selenite_core::vault::Vault;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    Capability, PluginConfig, PluginHost, PluginManifest, PluginRun, SandboxLimits, SignedDocument,
    config::{validate_id, validate_url},
    signed,
};

pub const CATALOG_SIGNING_DOMAIN: &[u8] = b"selenite.catalog.v1\n";
const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_RECORD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub manifest: PluginManifest,
    pub url: String,
    /// Lowercase SHA-256 hex of the exact binary WebAssembly download.
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub issued_at: u64,
    pub expires_at: u64,
    pub entries: Vec<CatalogEntry>,
}

/// Construction requires signature verification. Installation rechecks the
/// signature and expiry under the installer's current trusted-key settings.
pub struct VerifiedCatalog {
    catalog: Catalog,
    document: SignedDocument,
}

impl VerifiedCatalog {
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
}

pub fn verify_catalog(
    config: &PluginConfig,
    document: &SignedDocument,
    now: u64,
) -> Result<VerifiedCatalog> {
    config.validate()?;
    ensure!(config.enabled, "plugins are disabled");
    signed::verify(
        document,
        &config.trusted_catalog_keys,
        CATALOG_SIGNING_DOMAIN,
        MAX_CATALOG_BYTES,
    )?;
    let catalog: Catalog = serde_json::from_str(&document.payload)?;
    ensure!(
        catalog.issued_at <= now
            && now < catalog.expires_at
            && catalog.issued_at < catalog.expires_at,
        "catalog is expired or not yet valid"
    );
    ensure!(
        catalog.entries.len() <= 1000,
        "catalog has too many entries"
    );
    let mut ids = BTreeSet::new();
    for entry in &catalog.entries {
        entry.manifest.validate()?;
        validate_url(&entry.url)?;
        validate_digest(&entry.sha256)?;
        ensure!(
            ids.insert(&entry.manifest.id),
            "duplicate catalog plugin ID"
        );
    }
    Ok(VerifiedCatalog {
        catalog,
        document: document.clone(),
    })
}

pub struct CatalogClient {
    config: PluginConfig,
    client: Client,
}

impl CatalogClient {
    pub fn new(config: &PluginConfig) -> Result<Self> {
        config.validate()?;
        ensure!(config.enabled, "plugins are disabled");
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Selenite/0.1")
            .build()?;
        Ok(Self {
            config: config.clone(),
            client,
        })
    }

    /// Network access happens only when this explicit method is called.
    pub fn fetch_catalog(&self, now: u64) -> Result<VerifiedCatalog> {
        let url = self
            .config
            .catalog_url
            .as_deref()
            .context("plugin catalog URL is not configured")?;
        let bytes = self.download(url, MAX_CATALOG_BYTES * 2)?;
        let document: SignedDocument =
            serde_json::from_slice(&bytes).context("invalid signed catalog envelope")?;
        verify_catalog(&self.config, &document, now)
    }

    pub fn install(
        &self,
        vault: &Vault,
        catalog: &VerifiedCatalog,
        id: &str,
    ) -> Result<InstalledPlugin> {
        let verified = verify_catalog(&self.config, &catalog.document, now()?)?;
        let entry = verified
            .catalog
            .entries
            .iter()
            .find(|entry| entry.manifest.id == id)
            .context("plugin is absent from the verified catalog")?;
        let bytes = self.download(&entry.url, self.config.limits.max_module_bytes)?;
        self.install_from_bytes(vault, &verified, id, &bytes)
    }

    /// Useful for an explicitly selected local/offline package. The same
    /// verified catalog and exact digest are required as for an HTTP download.
    pub fn install_from_bytes(
        &self,
        vault: &Vault,
        catalog: &VerifiedCatalog,
        id: &str,
        bytes: &[u8],
    ) -> Result<InstalledPlugin> {
        let verified = verify_catalog(&self.config, &catalog.document, now()?)?;
        let entry = verified
            .catalog
            .entries
            .iter()
            .find(|entry| entry.manifest.id == id)
            .context("plugin is absent from the verified catalog")?;
        ensure!(
            bytes.len() <= self.config.limits.max_module_bytes && bytes.starts_with(b"\0asm"),
            "package is not a bounded binary WebAssembly module"
        );
        ensure!(
            digest(bytes) == entry.sha256,
            "downloaded package SHA-256 mismatch"
        );
        let package = Package {
            manifest: entry.manifest.clone(),
            sha256: entry.sha256.clone(),
            wasm_base64: STANDARD.encode(bytes),
            catalog: verified.document.clone(),
        };
        let package_source = serde_json::to_string(&package)?;
        let package_digest = digest(package_source.as_bytes());
        let package_path = package_path(id, &package_digest)?;
        match vault.read_metadata_bounded(&package_path, package_limit(&self.config.limits))? {
            Some(existing) => ensure!(
                existing == package_source,
                "stored package content does not match its address"
            ),
            None => match vault.write_metadata(&package_path, None, &package_source) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    ensure!(
                        vault
                            .read_metadata_bounded(
                                &package_path,
                                package_limit(&self.config.limits)
                            )?
                            .as_deref()
                            == Some(&package_source),
                        "concurrent package install differs"
                    );
                }
                Err(error) => return Err(error.into()),
            },
        }
        let plugin = InstalledPlugin {
            id: id.to_owned(),
            name: entry.manifest.name.clone(),
            version: entry.manifest.version.clone(),
            sha256: entry.sha256.clone(),
            capabilities: entry.manifest.capabilities.clone(),
        };
        let installed = InstalledRecord {
            plugin: plugin.clone(),
            package_sha256: package_digest,
        };
        let installed_path = installed_path(id)?;
        let previous = vault.read_metadata_bounded(&installed_path, MAX_RECORD_BYTES)?;
        vault.write_metadata(
            &installed_path,
            previous.as_deref(),
            &serde_json::to_string(&installed)?,
        )?;
        Ok(plugin)
    }

    fn download(&self, url: &str, limit: usize) -> Result<Vec<u8>> {
        let url = validate_url(url)?;
        let response = self.client.get(url).send()?.error_for_status()?;
        ensure!(
            response.status().is_success(),
            "catalog redirects are not followed"
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|length| length <= limit as u64),
            "download exceeds its size limit"
        );
        let mut bytes = Vec::new();
        response.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= limit, "download exceeds its size limit");
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub sha256: String,
    /// Requested capabilities only; installation never grants them.
    pub capabilities: BTreeSet<Capability>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledRecord {
    plugin: InstalledPlugin,
    package_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    manifest: PluginManifest,
    sha256: String,
    wasm_base64: String,
    catalog: SignedDocument,
}

pub fn list_installed(vault: &Vault) -> Result<Vec<InstalledPlugin>> {
    let _guard = vault.lock_shared()?;
    let mut plugins = Vec::new();
    let directory = Path::new("plugins").join("installed");
    for path in vault.metadata_paths(&directory)? {
        if path.parent() != Some(directory.as_path())
            || path.extension().and_then(|extension| extension.to_str()) != Some("json")
        {
            continue;
        }
        ensure!(plugins.len() < 1000, "too many installed plugins");
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .context("plugin filename is not Unicode")?;
        let record = read_record(vault, id)?;
        plugins.push(record.plugin);
    }
    plugins.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(plugins)
}

pub fn load_installed(
    vault: &Vault,
    id: &str,
    limits: &SandboxLimits,
) -> Result<(InstalledPlugin, PluginManifest, Vec<u8>)> {
    let (installed, package, wasm) = load_package(vault, id, limits)?;
    Ok((installed, package.manifest, wasm))
}

fn load_package(
    vault: &Vault,
    id: &str,
    limits: &SandboxLimits,
) -> Result<(InstalledPlugin, Package, Vec<u8>)> {
    limits.validate()?;
    let _guard = vault.lock_shared()?;
    let record = read_record(vault, id)?;
    let source = vault
        .read_metadata_bounded(
            package_path(id, &record.package_sha256)?,
            package_limit(limits),
        )?
        .context("installed plugin package is missing")?;
    ensure!(
        digest(source.as_bytes()) == record.package_sha256,
        "installed package was modified"
    );
    let package: Package = serde_json::from_str(&source)?;
    package.manifest.validate()?;
    ensure!(
        package.manifest.id == id
            && package.manifest.version == record.plugin.version
            && package.manifest.name == record.plugin.name
            && package.manifest.capabilities == record.plugin.capabilities
            && package.sha256 == record.plugin.sha256,
        "installed plugin metadata mismatch"
    );
    let wasm = STANDARD.decode(&package.wasm_base64)?;
    ensure!(
        wasm.len() <= limits.max_module_bytes
            && wasm.starts_with(b"\0asm")
            && digest(&wasm) == package.sha256,
        "installed WebAssembly integrity check failed"
    );
    Ok((record.plugin, package, wasm))
}

pub fn run_installed(
    vault: &Vault,
    config: &PluginConfig,
    id: &str,
    input: Value,
) -> Result<PluginRun> {
    run_selected(vault, config, id, None, input)
}

/// Execute the same installed manifest and module that the caller reviewed.
/// Validation and execution share the loaded bytes, so a concurrent install
/// cannot substitute a newer module after the review comparison succeeds.
pub fn run_installed_reviewed(
    vault: &Vault,
    config: &PluginConfig,
    expected: &InstalledPlugin,
    input: Value,
) -> Result<PluginRun> {
    run_selected(vault, config, &expected.id, Some(expected), input)
}

fn run_selected(
    vault: &Vault,
    config: &PluginConfig,
    id: &str,
    expected: Option<&InstalledPlugin>,
    input: Value,
) -> Result<PluginRun> {
    config.validate()?;
    ensure!(config.enabled, "plugins are disabled");
    let (installed, package, wasm) = load_package(vault, id, &config.limits)?;
    ensure!(
        expected.is_none_or(|expected| expected == &installed),
        "installed plugin changed; review its version and capabilities again"
    );
    // An installed plugin outlives its catalog's download window, but its
    // original signature must still verify under currently trusted keys.
    signed::verify(
        &package.catalog,
        &config.trusted_catalog_keys,
        CATALOG_SIGNING_DOMAIN,
        MAX_CATALOG_BYTES,
    )?;
    let catalog: Catalog = serde_json::from_str(&package.catalog.payload)?;
    ensure!(
        catalog
            .entries
            .iter()
            .any(|entry| entry.manifest == package.manifest && entry.sha256 == package.sha256),
        "installed package is not authorized by its signed catalog"
    );
    let grants = config.grants.get(id).cloned().unwrap_or_default();
    PluginHost::new(config)?.run(vault, &package.manifest, &wasm, &grants, input)
}

fn read_record(vault: &Vault, id: &str) -> Result<InstalledRecord> {
    let source = vault
        .read_metadata_bounded(installed_path(id)?, MAX_RECORD_BYTES)?
        .context("plugin is not installed")?;
    let record: InstalledRecord = serde_json::from_str(&source)?;
    ensure!(record.plugin.id == id, "installed plugin ID mismatch");
    validate_digest(&record.plugin.sha256)?;
    validate_digest(&record.package_sha256)?;
    Ok(record)
}

fn installed_path(id: &str) -> Result<PathBuf> {
    validate_id(id)?;
    Ok(Path::new("plugins")
        .join("installed")
        .join(format!("{id}.json")))
}

fn package_path(id: &str, sha256: &str) -> Result<PathBuf> {
    validate_id(id)?;
    validate_digest(sha256)?;
    Ok(Path::new("plugins")
        .join("packages")
        .join(id)
        .join(format!("{sha256}.json")))
}

fn package_limit(limits: &SandboxLimits) -> usize {
    limits.max_module_bytes * 2 + MAX_CATALOG_BYTES * 2 + MAX_RECORD_BYTES
}

fn validate_digest(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
        "SHA-256 must be 64 lowercase hexadecimal characters"
    );
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
