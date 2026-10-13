//! Optional signed entitlements. Nothing in the community engine is gated by
//! this verifier; a deployment defines its own additional capability names.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{SignedDocument, signed};

pub const ENTITLEMENT_SIGNING_DOMAIN: &[u8] = b"selenite.entitlement.v1\n";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProConfig {
    pub trusted_keys: BTreeMap<String, String>,
    pub issuer: Option<String>,
    pub audience: Option<String>,
}

impl ProConfig {
    pub fn validate(&self) -> Result<()> {
        signed::validate_keys(&self.trusted_keys)?;
        for value in self.issuer.iter().chain(&self.audience) {
            ensure!(
                !value.is_empty() && value.len() <= 256,
                "issuer and audience must contain 1–256 bytes"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entitlement {
    pub subject: String,
    pub issuer: String,
    pub audience: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pub capabilities: BTreeSet<String>,
}

impl Entitlement {
    pub fn has_capability(&self, capability: &str, now: u64) -> bool {
        self.issued_at <= now && now < self.expires_at && self.capabilities.contains(capability)
    }
}

pub fn verify_entitlement(
    config: &ProConfig,
    document: &SignedDocument,
    now: u64,
) -> Result<Entitlement> {
    config.validate()?;
    signed::verify(
        document,
        &config.trusted_keys,
        ENTITLEMENT_SIGNING_DOMAIN,
        16 * 1024,
    )?;
    let entitlement: Entitlement = serde_json::from_str(&document.payload)?;
    ensure!(
        !entitlement.subject.is_empty() && entitlement.subject.len() <= 256,
        "invalid entitlement subject"
    );
    ensure!(
        !entitlement.issuer.is_empty()
            && entitlement.issuer.len() <= 256
            && !entitlement.audience.is_empty()
            && entitlement.audience.len() <= 256,
        "invalid entitlement issuer or audience"
    );
    ensure!(
        config
            .issuer
            .as_ref()
            .is_none_or(|issuer| issuer == &entitlement.issuer),
        "entitlement issuer mismatch"
    );
    ensure!(
        config
            .audience
            .as_ref()
            .is_none_or(|audience| audience == &entitlement.audience),
        "entitlement audience mismatch"
    );
    ensure!(
        entitlement.issued_at <= now
            && now < entitlement.expires_at
            && entitlement.issued_at < entitlement.expires_at,
        "entitlement is expired or not yet valid"
    );
    ensure!(
        entitlement.capabilities.len() <= 128,
        "too many entitlement capabilities"
    );
    for name in &entitlement.capabilities {
        ensure!(
            !name.is_empty()
                && name.len() <= 128
                && name.bytes().all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'_' | b'-' | b':')),
            "invalid entitlement capability name"
        );
    }
    Ok(entitlement)
}
