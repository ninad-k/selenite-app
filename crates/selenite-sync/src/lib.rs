//! Reviewed, conflict-preserving sync over append-only remote version archives.
//! No provider operation deletes or replaces an existing remote object.

mod providers;
mod state;

pub use providers::{GoogleDrive, OneDrive, configured_provider};
pub use state::{
    SyncAction, SyncActionKind, SyncIssue, SyncPlan, SyncReport, plan, resolve_conflict, run,
};

use anyhow::{Context, Result, bail, ensure};
use selenite_core::{
    Page,
    vault::{Vault, validate_relative},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use url::Url;

pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_VERSION_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_REMOTE_VERSIONS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    GoogleDrive,
    OneDrive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyncConfig {
    pub enabled: bool,
    pub provider: ProviderKind,
    pub endpoint: String,
    pub upload_endpoint: Option<String>,
    pub token_env: String,
    pub folder_id: String,
    pub drive_id: Option<String>,
    pub include_metadata: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: ProviderKind::GoogleDrive,
            endpoint: "https://www.googleapis.com/drive/v3/".into(),
            upload_endpoint: None,
            token_env: "SELENITE_SYNC_TOKEN".into(),
            folder_id: String::new(),
            drive_id: None,
            include_metadata: false,
        }
    }
}

impl SyncConfig {
    pub fn load(vault: &Vault) -> Result<Self> {
        let config = match vault.read_metadata_bounded("sync.toml", 64 * 1024)? {
            Some(source) => toml::from_str(&source).context("invalid sync.toml")?,
            None => Self::default(),
        };
        if config.enabled {
            config.validate()?;
        }
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        endpoint(&self.endpoint)?;
        if let Some(upload) = &self.upload_endpoint {
            endpoint(upload)?;
        }
        ensure!(
            self.provider == ProviderKind::GoogleDrive || self.upload_endpoint.is_none(),
            "upload_endpoint applies only to Google Drive; Graph supplies a preauthenticated upload URL"
        );
        ensure!(
            (!self.enabled && self.folder_id.is_empty())
                || (!matches!(self.folder_id.as_str(), "" | "." | "..")
                    && self
                        .folder_id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '!' | '.'))),
            "folder_id must be an opaque provider item ID"
        );
        if let Some(drive) = &self.drive_id {
            ensure!(
                !matches!(drive.as_str(), "" | "." | "..")
                    && drive
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '!' | '.')),
                "invalid drive_id"
            );
        }
        ensure!(
            !self.token_env.is_empty()
                && self
                    .token_env
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "token_env must name an environment variable"
        );
        Ok(())
    }
}

/// A transport must preserve every accepted version. `publish` must create an
/// immutable object, or verify an identical existing object, never overwrite.
/// Concurrent roots/children are intentional and remain visible in `versions`.
pub trait SyncProvider: Send + Sync {
    fn name(&self) -> &str;
    fn identity(&self) -> String;
    fn versions(&self) -> Result<Vec<RemoteVersion>>;
    fn publish(&self, version: &RemoteVersion) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteVersion {
    pub format: u32,
    pub revision: String,
    pub path: String,
    pub parents: Vec<String>,
    pub source: String,
}

impl RemoteVersion {
    pub fn new(path: &str, mut parents: Vec<String>, source: &str) -> Result<Self> {
        validate_source(path, source)?;
        parents.sort();
        parents.dedup();
        ensure!(
            parents.iter().all(|parent| valid_hash(parent)),
            "invalid parent revision"
        );
        let revision = digest(serde_json::to_vec(&(1, path, &parents, source))?);
        Ok(Self {
            format: 1,
            revision,
            path: path.into(),
            parents,
            source: source.into(),
        })
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.format == 1, "unsupported sync version format");
        ensure!(
            self == &Self::new(&self.path, self.parents.clone(), &self.source)?,
            "sync version checksum or parent ordering is invalid"
        );
        Ok(())
    }
}

pub(crate) fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn endpoint(value: &str) -> Result<Url> {
    let mut url = Url::parse(value).context("invalid sync endpoint")?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "sync endpoints cannot contain credentials, query, or fragment"
    );
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "sync requires HTTPS (HTTP allowed only for loopback tests)"
    );
    ensure!(url.host_str().is_some(), "sync endpoint needs a host");
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

pub(crate) fn logical_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn content_path(path: &str) -> Result<(PathBuf, bool)> {
    ensure!(
        !path.contains('\\'),
        "sync paths use slash-separated portable components"
    );
    if let Some(metadata) = path.strip_prefix(".selenite/") {
        let relative: PathBuf = metadata.split('/').collect();
        ensure!(
            logical_path(&relative) == metadata,
            "sync metadata path must be canonical"
        );
        validate_relative(&relative)?;
        let mut parts = metadata.split('/');
        let first = parts.next().unwrap_or_default();
        ensure!(parts.next().is_some(), "metadata requires a filename");
        let extension = relative
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        ensure!(
            (matches!(first, "schemas" | "views") && extension == "toml")
                || (first == "templates" && extension == "md"),
            "metadata path is outside the authoritative sync allowlist"
        );
        Ok((relative, true))
    } else {
        let relative: PathBuf = path.split('/').collect();
        ensure!(
            logical_path(&relative) == path,
            "sync page path must be canonical"
        );
        selenite_core::validate_page_path(&relative)?;
        Ok((relative, false))
    }
}

pub(crate) fn validate_source(path: &str, source: &str) -> Result<()> {
    ensure!(
        source.len() <= MAX_SOURCE_BYTES,
        "sync source exceeds 8 MiB"
    );
    let (relative, metadata) = content_path(path)?;
    if metadata
        && relative
            .extension()
            .is_some_and(|extension| extension == "toml")
    {
        let _: toml::Value =
            toml::from_str(source).context("invalid authoritative TOML metadata")?;
    } else {
        Page::parse(&relative, source).context("invalid Markdown sync source")?;
    }
    Ok(())
}

pub(crate) fn read_content(vault: &Vault, path: &str) -> Result<Option<String>> {
    let (relative, metadata) = content_path(path)?;
    if metadata {
        return Ok(vault.read_metadata_bounded(relative, MAX_SOURCE_BYTES)?);
    }
    match vault.read_source_bounded(relative, MAX_SOURCE_BYTES) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn apply_content(
    vault: &Vault,
    path: &str,
    expected: Option<&str>,
    source: &str,
) -> Result<()> {
    validate_source(path, source)?;
    let (relative, metadata) = content_path(path)?;
    if metadata {
        vault.write_metadata(relative, expected, source)?;
    } else if let Some(expected) = expected {
        vault.write_page(relative, expected, source)?;
    } else if !vault.create_page(&relative, source)? {
        bail!("local create conflict at {path}");
    }
    Ok(())
}
