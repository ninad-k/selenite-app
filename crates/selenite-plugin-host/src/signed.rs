use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Sign domain-prefix bytes followed by the exact UTF-8 payload. The outer JSON
/// formatting is irrelevant; no canonicalization or reserialization is needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedDocument {
    pub key_id: String,
    pub payload: String,
    pub signature: String,
}

pub(crate) fn validate_keys(keys: &BTreeMap<String, String>) -> Result<()> {
    ensure!(
        keys.len() <= 64,
        "at most 64 trusted public keys are supported"
    );
    for (id, value) in keys {
        ensure!(
            !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control),
            "invalid signing key ID"
        );
        public_key(value).with_context(|| format!("invalid public key {id}"))?;
    }
    Ok(())
}

fn public_key(encoded: &str) -> Result<VerifyingKey> {
    ensure!(
        encoded.len() == 44,
        "public key must be a base64-encoded 32-byte key"
    );
    let bytes: [u8; 32] = STANDARD
        .decode(encoded)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("public key must be 32 bytes"))?;
    VerifyingKey::from_bytes(&bytes).context("invalid Ed25519 public key")
}

pub(crate) fn verify(
    document: &SignedDocument,
    keys: &BTreeMap<String, String>,
    domain: &[u8],
    max_bytes: usize,
) -> Result<()> {
    ensure!(
        document.payload.len() <= max_bytes,
        "signed payload exceeds its size limit"
    );
    ensure!(
        document.signature.len() == 88,
        "signature must be a base64-encoded 64-byte signature"
    );
    let encoded_key = keys
        .get(&document.key_id)
        .context("signing key is not trusted")?;
    let key = public_key(encoded_key)?;
    let signature = Signature::from_slice(&STANDARD.decode(&document.signature)?)?;
    let mut message = Vec::with_capacity(domain.len() + document.payload.len());
    message.extend_from_slice(domain);
    message.extend_from_slice(document.payload.as_bytes());
    key.verify_strict(&message, &signature)
        .context("invalid signature")
}
