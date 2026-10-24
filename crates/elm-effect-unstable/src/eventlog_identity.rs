//! Identity root-secret derivation (`eventlog/internal/identityRootSecretDerivation`).
//!
//! Derives encryption + signing material from an identity's root secret via
//! SHA-256 over a domain-separated, versioned label. Memoized per identity
//! value through [`GetIdentityRootSecretMaterial`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

pub const ENCRYPTION_LABEL: &str = "effect/eventlog/identity/v1/encryption";
pub const SIGNING_LABEL: &str = "effect/eventlog/identity/v1/signing";

const ED25519_PKCS8_SEED_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub public_key: String,
    pub root_secret: Vec<u8>,
}

#[derive(Clone)]
pub struct IdentityRootSecretMaterial {
    pub encryption_key_material: Vec<u8>,
    pub signing_public_key: Vec<u8>,
    pub signing_private_key: Vec<u8>,
}

fn derive_secret_bytes(root_secret: &[u8], label: &str) -> Vec<u8> {
    let label_bytes = label.as_bytes();
    let mut input = Vec::with_capacity(label_bytes.len() + 1 + root_secret.len());
    input.extend_from_slice(label_bytes);
    input.push(0);
    input.extend_from_slice(root_secret);
    Sha256::digest(&input).to_vec()
}

pub fn derive_identity_root_secret_material(root_secret: &[u8]) -> IdentityRootSecretMaterial {
    let encryption_key_material = derive_secret_bytes(root_secret, ENCRYPTION_LABEL);
    let signing_seed = derive_secret_bytes(root_secret, SIGNING_LABEL);

    let mut signing_private_key =
        Vec::with_capacity(ED25519_PKCS8_SEED_PREFIX.len() + signing_seed.len());
    signing_private_key.extend_from_slice(&ED25519_PKCS8_SEED_PREFIX);
    signing_private_key.extend_from_slice(&signing_seed);

    let signing_key = SigningKey::from_bytes(signing_seed.as_slice().try_into().expect("32 bytes"));
    let signing_public_key = signing_key.verifying_key().to_bytes().to_vec();

    IdentityRootSecretMaterial {
        encryption_key_material,
        signing_public_key,
        signing_private_key,
    }
}

/// `makeGetIdentityRootSecretMaterial`: the derivation memoized across calls.
///
/// The pin caches on the identity object with a `WeakMap`; an identity here is
/// a value, so the cache keys on its two fields. One instance asked twice gets
/// the same `Arc`, and a second instance with the same secret gets the same
/// answer, because the derivation is a pure function of the secret.
pub struct GetIdentityRootSecretMaterial {
    cache: Mutex<BTreeMap<(String, Vec<u8>), Arc<IdentityRootSecretMaterial>>>,
}

impl GetIdentityRootSecretMaterial {
    /// An empty cache.
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// The material for `identity`, derived once and reused after that.
    pub fn call(&self, identity: &Identity) -> Arc<IdentityRootSecretMaterial> {
        let key = (identity.public_key.clone(), identity.root_secret.clone());
        if let Some(cached) = self.cache.lock().unwrap().get(&key) {
            return Arc::clone(cached);
        }
        let derived = Arc::new(derive_identity_root_secret_material(&identity.root_secret));
        self.cache
            .lock()
            .unwrap()
            .insert(key, Arc::clone(&derived));
        derived
    }
}

impl Default for GetIdentityRootSecretMaterial {
    fn default() -> Self {
        Self::new()
    }
}

/// A fresh memoizing derivation, the pin's factory.
pub fn make_get_identity_root_secret_material() -> GetIdentityRootSecretMaterial {
    GetIdentityRootSecretMaterial::new()
}
