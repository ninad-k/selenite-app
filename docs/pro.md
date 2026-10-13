# Configurable Pro entitlement verification

Community operations remain available without an entitlement. The verifier does not gate notes, journaling, databases, tasks, search, sync, plugins, AI or MCP. It introduces no paid-feature list, checkout flow, payment backend, account service or deployed signing endpoint. A future deployment may define additional capability names and its own issuer; this code verifies its signed claims locally.

The desktop and CLI read `.selenite/pro.toml`. Its public `ProConfig` type accepts configured Ed25519 public keys plus optional expected issuer and audience values:

```toml
# issuer = "your-publisher"
# audience = "selenite"

[trusted_keys]
# publisher_key_id = "base64-encoded-32-byte-Ed25519-public-key"
```

An empty key set trusts no issuer. Keys come from user/deployment configuration, never from the submitted entitlement. Private signing keys must stay with the issuer; Selenite needs only public keys.

The submitted `SignedDocument` has `key_id`, `payload` and `signature` strings, using the same envelope format as the [plugin catalog](plugins.md#signed-catalog-and-package-installation). Its domain prefix is different: sign the UTF-8 bytes `selenite.entitlement.v1\n` followed by the exact UTF-8 payload bytes. The sequence `\n` denotes one newline byte. Domain separation prevents a catalog signature from being interpreted as an entitlement signature.

The payload is JSON with this shape:

```json
{
  "subject": "issuer-defined-subject",
  "issuer": "your-publisher",
  "audience": "selenite",
  "issued_at": 1791849600,
  "expires_at": 1791936000,
  "capabilities": []
}
```

Times are Unix seconds. `verify_entitlement(config, document, now)` validates the configuration, strict Ed25519 signature, bounded payload and capability names, subject, configured issuer/audience, and `issued_at <= now < expires_at`. Invalid, expired or not-yet-valid documents return an error. The returned `Entitlement::has_capability(name, now)` rechecks the validity window when testing a capability; a caller must pass the current time at use, not reuse an old verification timestamp.

The mechanism depends on the local clock and configured trust keys. It provides no online revocation, tamper-resistant clock, device binding, billing state or automatic renewal. Applications must not treat a deserialized unverified claim as an entitlement. Keeping these limits explicit avoids claiming a hosted licensing system that does not exist.

Tests cover valid claims, tampering, expiry boundaries, future issue times, wrong issuer/audience and a wrong signing domain. The dependency used for signature verification is `ed25519-dalek` 2.2.0 (BSD-3-Clause); the complete shared dependency rationale is in [Plugin hosting](plugins.md#dependency-choices).
