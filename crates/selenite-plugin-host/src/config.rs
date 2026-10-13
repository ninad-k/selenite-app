use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::signed::validate_keys;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    VaultList,
    VaultRead,
    VaultWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub abi_version: u32,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
}

impl PluginManifest {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 200,
            "plugin name must contain 1–200 bytes"
        );
        ensure!(
            !self.version.is_empty()
                && self.version.len() <= 64
                && self
                    .version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+')),
            "invalid plugin version"
        );
        ensure!(self.abi_version == 1, "unsupported plugin ABI version");
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxLimits {
    pub fuel: u64,
    pub memory_bytes: usize,
    pub max_module_bytes: usize,
    pub max_message_bytes: usize,
    pub max_host_calls: u32,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            fuel: 5_000_000,
            memory_bytes: 16 * 1024 * 1024,
            max_module_bytes: 2 * 1024 * 1024,
            max_message_bytes: 1024 * 1024,
            max_host_calls: 64,
        }
    }
}

impl SandboxLimits {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=100_000_000).contains(&self.fuel),
            "fuel must be 1–100000000"
        );
        ensure!(
            (64 * 1024..=64 * 1024 * 1024).contains(&self.memory_bytes),
            "memory limit must be 64 KiB–64 MiB"
        );
        ensure!(
            (8..=8 * 1024 * 1024).contains(&self.max_module_bytes),
            "module limit must be 8 bytes–8 MiB"
        );
        ensure!(
            (1024..=2 * 1024 * 1024).contains(&self.max_message_bytes)
                && self.max_message_bytes <= self.memory_bytes / 2,
            "message limit must be 1 KiB–2 MiB and at most half the guest memory"
        );
        ensure!(
            (1..=256).contains(&self.max_host_calls),
            "host call limit must be 1–256"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginConfig {
    pub enabled: bool,
    pub catalog_url: Option<String>,
    /// Key ID to a base64-encoded 32-byte Ed25519 public key.
    pub trusted_catalog_keys: BTreeMap<String, String>,
    /// Explicit per-plugin grants. Merely installing a plugin grants nothing.
    pub grants: BTreeMap<String, BTreeSet<Capability>>,
    pub limits: SandboxLimits,
}

impl PluginConfig {
    pub fn validate(&self) -> Result<()> {
        self.limits.validate()?;
        if let Some(url) = &self.catalog_url {
            validate_url(url)?;
        }
        validate_keys(&self.trusted_catalog_keys)?;
        for id in self.grants.keys() {
            validate_id(id)?;
        }
        Ok(())
    }
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_')),
        "plugin ID must contain 1–64 lowercase ASCII letters, digits, hyphens or underscores"
    );
    ensure!(
        !matches!(id, "con" | "prn" | "aux" | "nul")
            && !(id.len() == 4
                && (id.starts_with("com") || id.starts_with("lpt"))
                && matches!(id.as_bytes()[3], b'1'..=b'9')),
        "plugin ID is a reserved Windows filename"
    );
    Ok(())
}

pub(crate) fn validate_url(value: &str) -> Result<url::Url> {
    ensure!(value.len() <= 4096, "URL exceeds 4096 bytes");
    let url = url::Url::parse(value).context("invalid service URL")?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(domain)) => domain == "localhost",
        None => false,
    };
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "service URL must use HTTPS (HTTP is allowed only for loopback testing)"
    );
    ensure!(
        url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "service URL needs a host and must not embed credentials or a fragment"
    );
    Ok(url)
}
