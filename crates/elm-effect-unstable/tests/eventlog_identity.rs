#![cfg(feature = "eventlog")]

use std::sync::Arc;

use elm_effect_unstable::eventlog_identity::{
    make_get_identity_root_secret_material, Identity, ENCRYPTION_LABEL, SIGNING_LABEL,
};

fn make_root_secret() -> Vec<u8> {
    (0..32).map(|i| ((i * 17) % 256) as u8).collect()
}

fn make_identity(root_secret: Vec<u8>, public_key: &str) -> Identity {
    Identity { public_key: public_key.to_owned(), root_secret }
}

#[test]
fn derives_same_material_for_same_root_secret() {
    let root_secret = make_root_secret();
    let derive = make_get_identity_root_secret_material();
    let first = derive.call(&make_identity(root_secret.clone(), "user-a"));
    let second = derive.call(&make_identity(root_secret, "user-b"));
    assert!(ENCRYPTION_LABEL.contains("/v1/"));
    assert!(SIGNING_LABEL.contains("/v1/"));
    assert_eq!(first.encryption_key_material, second.encryption_key_material);
    assert_eq!(first.signing_public_key, second.signing_public_key);
    assert_eq!(first.signing_private_key, second.signing_private_key);
}

#[test]
fn uses_versioned_domain_separated_labels() {
    let root_secret = make_root_secret();
    let derive = make_get_identity_root_secret_material();
    let derived = derive.call(&make_identity(root_secret, "user-a"));
    // signing seed is last 32 bytes of pkcs8
    let signing_seed = &derived.signing_private_key[derived.signing_private_key.len() - 32..];
    assert_ne!(derived.encryption_key_material.as_slice(), signing_seed);
    assert_ne!(ENCRYPTION_LABEL, SIGNING_LABEL);
}

#[test]
fn memoizes_for_same_identity_instance() {
    let root_secret = make_root_secret();
    let derive = make_get_identity_root_secret_material();
    let identity = make_identity(root_secret, "user-a");
    let first = derive.call(&identity);
    let second = derive.call(&identity);
    assert!(Arc::ptr_eq(&first, &second));
}