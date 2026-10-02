//! Portable, signed identity for a prebuilt Ayni environment image.
//!
//! The certificate intentionally does not contain an OCI manifest digest: an
//! in-image digest would change that image's digest. Launchers may bind an
//! independently observed OCI digest to their execution evidence.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Wire version for portable environment certificates.
pub const ENVIRONMENT_CERTIFICATE_SCHEMA_VERSION: &str = "2";

/// Claims made by a builder about protected, prebuilt environment content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCertificate {
    pub schema_version: String,
    pub lock_fingerprint: String,
    pub platform: String,
    pub ayni_version: String,
    pub tool_inventory_digest: String,
    pub protected_content_root: String,
}

impl EnvironmentCertificate {
    /// Constructs a normalized certificate payload.
    pub fn new(
        lock_fingerprint: impl Into<String>,
        platform: impl Into<String>,
        ayni_version: impl Into<String>,
        tool_inventory_digest: impl Into<String>,
        protected_content_root: impl Into<String>,
    ) -> Result<Self, CertificateError> {
        let certificate = Self {
            schema_version: ENVIRONMENT_CERTIFICATE_SCHEMA_VERSION.into(),
            lock_fingerprint: lock_fingerprint.into(),
            platform: platform.into(),
            ayni_version: ayni_version.into(),
            tool_inventory_digest: tool_inventory_digest.into(),
            protected_content_root: protected_content_root.into(),
        };
        certificate.validate()?;
        Ok(certificate)
    }

    /// Returns canonical bytes used as the Ed25519 signing message.
    pub fn canonical_payload(&self) -> Result<Vec<u8>, CertificateError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| CertificateError::Serialization(error.to_string()))
    }

    fn validate(&self) -> Result<(), CertificateError> {
        if self.schema_version != ENVIRONMENT_CERTIFICATE_SCHEMA_VERSION {
            return Err(CertificateError::UnsupportedSchema(
                self.schema_version.clone(),
            ));
        }
        validate_digest("lock fingerprint", &self.lock_fingerprint)?;
        validate_label("platform", &self.platform)?;
        validate_label("Ayni version", &self.ayni_version)?;
        validate_digest("tool inventory digest", &self.tool_inventory_digest)?;
        validate_digest("protected content root", &self.protected_content_root)?;
        Ok(())
    }
}

/// Certificate payload, signing key identifier, and detached Ed25519 signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCertificateEnvelope {
    pub certificate: EnvironmentCertificate,
    pub key_id: String,
    /// Lowercase hexadecimal Ed25519 signature over `certificate.canonical_payload()`.
    pub signature: String,
}

impl EnvironmentCertificateEnvelope {
    /// Creates metadata for a locally consistent image without asserting a
    /// builder identity.
    pub fn unsigned(certificate: EnvironmentCertificate) -> Result<Self, CertificateError> {
        certificate.canonical_payload()?;
        Ok(Self {
            certificate,
            key_id: String::from("unsigned"),
            signature: String::new(),
        })
    }
    pub fn sign(
        certificate: EnvironmentCertificate,
        key_id: impl Into<String>,
        key: &SigningKey,
    ) -> Result<Self, CertificateError> {
        let key_id = key_id.into();
        validate_key_id(&key_id)?;
        let signature = key.sign(&certificate.canonical_payload()?).to_bytes();
        Ok(Self {
            certificate,
            key_id,
            signature: crate::lower_hex(signature),
        })
    }

    /// Verifies the signature against a key pinned by the trust policy.
    pub fn verify(
        &self,
        trust: &EnvironmentCertificateTrustPolicy,
    ) -> Result<(), CertificateError> {
        if self.key_id == "unsigned" && self.signature.is_empty() {
            return Ok(());
        }
        if self.key_id == "unsigned" || self.signature.is_empty() {
            return Err(CertificateError::InvalidSignature);
        }
        validate_key_id(&self.key_id)?;
        let public_key = trust
            .trusted_keys
            .get(&self.key_id)
            .ok_or_else(|| CertificateError::UntrustedKey(self.key_id.clone()))?;
        let key = decode_public_key(public_key)?;
        let signature =
            Signature::from_bytes(&decode_fixed_hex::<64>("signature", &self.signature)?);
        key.verify(&self.certificate.canonical_payload()?, &signature)
            .map_err(|_| CertificateError::InvalidSignature)
    }
}

/// Repository-pinned keys accepted for portable environment certificates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentCertificateTrustPolicy {
    trusted_keys: BTreeMap<String, String>,
}

impl EnvironmentCertificateTrustPolicy {
    pub fn new(trusted_keys: BTreeMap<String, String>) -> Result<Self, CertificateError> {
        if trusted_keys.is_empty() {
            return Ok(Self { trusted_keys });
        }
        for (key_id, public_key) in &trusted_keys {
            validate_key_id(key_id)?;
            decode_public_key(public_key)?;
        }
        Ok(Self { trusted_keys })
    }

    #[must_use]
    pub fn fingerprint(&self) -> String {
        crate::sha256_fingerprint(
            serde_json::to_vec(&self.trusted_keys).expect("trusted keys serialize"),
        )
    }

    /// Whether this exact public key is pinned under the supplied key identifier.
    #[must_use]
    pub fn trusts_key(&self, key_id: &str, public_key: &[u8; 32]) -> bool {
        self.trusted_keys
            .get(key_id)
            .is_some_and(|value| value == &crate::lower_hex(public_key))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trusted_keys.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertificateError {
    UnsupportedSchema(String),
    InvalidField { field: &'static str, value: String },
    UntrustedKey(String),
    InvalidSignature,
    Serialization(String),
}

impl fmt::Display for CertificateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => write!(
                formatter,
                "unsupported environment certificate schema '{version}'"
            ),
            Self::InvalidField { field, value } => write!(
                formatter,
                "invalid environment certificate {field}: '{value}'"
            ),
            Self::UntrustedKey(key_id) => write!(
                formatter,
                "environment certificate key '{key_id}' is not trusted"
            ),
            Self::InvalidSignature => {
                formatter.write_str("environment certificate signature is invalid")
            }
            Self::Serialization(error) => write!(
                formatter,
                "failed to serialize environment certificate: {error}"
            ),
        }
    }
}

impl std::error::Error for CertificateError {}

fn validate_label(field: &'static str, value: &str) -> Result<(), CertificateError> {
    if value.trim().is_empty() || value != value.trim() || value.contains(char::is_whitespace) {
        return Err(CertificateError::InvalidField {
            field,
            value: value.into(),
        });
    }
    Ok(())
}

fn validate_key_id(value: &str) -> Result<(), CertificateError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CertificateError::InvalidField {
            field: "key id",
            value: value.into(),
        });
    }
    Ok(())
}

fn validate_digest(field: &'static str, value: &str) -> Result<(), CertificateError> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(CertificateError::InvalidField {
            field,
            value: value.into(),
        });
    }
    Ok(())
}

fn decode_public_key(value: &str) -> Result<VerifyingKey, CertificateError> {
    VerifyingKey::from_bytes(&decode_fixed_hex::<32>("public key", value)?).map_err(|_| {
        CertificateError::InvalidField {
            field: "public key",
            value: value.into(),
        }
    })
}

fn decode_fixed_hex<const LENGTH: usize>(
    field: &'static str,
    value: &str,
) -> Result<[u8; LENGTH], CertificateError> {
    if value.len() != LENGTH * 2 {
        return Err(CertificateError::InvalidField {
            field,
            value: value.into(),
        });
    }
    let mut decoded = [0; LENGTH];
    for (index, output) in decoded.iter_mut().enumerate() {
        let part = &value[index * 2..index * 2 + 2];
        *output = u8::from_str_radix(part, 16).map_err(|_| CertificateError::InvalidField {
            field,
            value: value.into(),
        })?;
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certificate() -> EnvironmentCertificate {
        EnvironmentCertificate::new(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "linux/amd64",
            "0.13.0",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        )
        .unwrap()
    }

    #[test]
    fn verifies_canonical_signed_certificate() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let mut keys = BTreeMap::new();
        keys.insert(
            "release-2026".into(),
            crate::lower_hex(signing_key.verifying_key().to_bytes()),
        );
        let envelope =
            EnvironmentCertificateEnvelope::sign(certificate(), "release-2026", &signing_key)
                .unwrap();
        let trust = EnvironmentCertificateTrustPolicy::new(keys).unwrap();
        assert!(trust.trusts_key("release-2026", &signing_key.verifying_key().to_bytes()));
        assert!(!trust.trusts_key("other", &signing_key.verifying_key().to_bytes()));
        assert!(!trust.trusts_key(
            "release-2026",
            &SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes()
        ));
        envelope.verify(&trust).unwrap();
    }

    #[test]
    fn rejects_tampered_payload_and_untrusted_keys() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let mut keys = BTreeMap::new();
        keys.insert(
            "release-2026".into(),
            crate::lower_hex(signing_key.verifying_key().to_bytes()),
        );
        let trust = EnvironmentCertificateTrustPolicy::new(keys).unwrap();
        let mut envelope =
            EnvironmentCertificateEnvelope::sign(certificate(), "release-2026", &signing_key)
                .unwrap();
        envelope.certificate.platform = "linux/arm64".into();
        assert_eq!(
            envelope.verify(&trust),
            Err(CertificateError::InvalidSignature)
        );
        envelope.key_id = "other".into();
        assert_eq!(
            envelope.verify(&trust),
            Err(CertificateError::UntrustedKey("other".into()))
        );
    }

    #[test]
    fn rejects_unsupported_schema_and_invalid_public_key() {
        let mut value = certificate();
        value.schema_version = "0".into();
        assert!(value.canonical_payload().is_err());
        let mut keys = BTreeMap::new();
        keys.insert("release".into(), "not-a-key".into());
        assert!(EnvironmentCertificateTrustPolicy::new(keys).is_err());
    }
}
