//! Behavior port for `eventlog/EventLogSessionAuth.test.ts`: the canonical
//! length-framed payload and its Ed25519 sign/verify surface.
//!
//! The pin's test builds a keypair with `WebCrypto` and exports the raw public
//! key plus PKCS#8 private key; here the same keypair is built from a seed, so
//! the bytes the tests exercise are the same shape.

#![cfg(feature = "eventlog")]

use ed25519_dalek::{SigningKey, VerifyingKey};
use elm_effect_unstable::eventlog_session_auth::{
    AUTH_PAYLOAD_CONTEXT, ED25519_PUBLIC_KEY_LENGTH, ED25519_SIGNATURE_LENGTH, RemoteId,
    SessionAuthErrorReason, SessionAuthPayload, decode_session_auth_payload,
    encode_session_auth_payload, sign_session_auth_payload, verify_session_auth_payload,
    verify_session_auth_payload_bytes,
};

/// The 16-byte PKCS#8 framing `WebCrypto` puts in front of an Ed25519 seed.
const ED25519_PKCS8_SEED_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

fn make_signing_key_pair() -> (Vec<u8>, Vec<u8>) {
    let seed: Vec<u8> = (0..32u8)
        .map(|i| i.wrapping_mul(7).wrapping_add(3))
        .collect();
    let signing_key = SigningKey::from_bytes(seed.as_slice().try_into().expect("32 bytes"));
    let public = signing_key.verifying_key().to_bytes().to_vec();

    let mut private = Vec::with_capacity(ED25519_PKCS8_SEED_PREFIX.len() + seed.len());
    private.extend_from_slice(&ED25519_PKCS8_SEED_PREFIX);
    private.extend_from_slice(&seed);
    (public, private)
}

fn make_payload(signing_public_key: Vec<u8>) -> SessionAuthPayload {
    SessionAuthPayload {
        remote_id: RemoteId::Text("remote-123".to_string()),
        challenge: vec![1, 2, 3, 4, 5, 6, 7, 8],
        public_key: "client-public-key".to_string(),
        signing_public_key,
    }
}

/// `EventLogSessionAuth.test.ts:26` — encodes and decodes canonical auth
/// payload deterministically.
#[test]
fn encode_and_decode_canonical_auth_payload_deterministically() {
    let signing_public_key: Vec<u8> = (0..ED25519_PUBLIC_KEY_LENGTH as u8).collect();
    let payload = make_payload(signing_public_key.clone());

    let encoded_a = encode_session_auth_payload(&payload).expect("payload encodes");
    let encoded_b = encode_session_auth_payload(&payload).expect("payload encodes");
    assert_eq!(encoded_a, encoded_b);

    // The canonical form is the protocol, not merely self-consistent: the first
    // framed field is the context, so a signature cannot be replayed into
    // another protocol.
    let mut expected_prefix = (AUTH_PAYLOAD_CONTEXT.len() as u32).to_be_bytes().to_vec();
    expected_prefix.extend_from_slice(AUTH_PAYLOAD_CONTEXT.as_bytes());
    assert_eq!(&encoded_a[..expected_prefix.len()], &expected_prefix[..]);

    let decoded = decode_session_auth_payload(&encoded_a).expect("payload decodes");
    assert_eq!(decoded.remote_id, payload.remote_id);
    assert_eq!(decoded.public_key, payload.public_key);
    assert_eq!(decoded.challenge, payload.challenge);
    assert_eq!(decoded.signing_public_key, payload.signing_public_key);
}

/// `EventLogSessionAuth.test.ts:46` — signs and verifies canonical auth payload
/// with Ed25519.
#[test]
fn signs_and_verifies_canonical_auth_payload_with_ed25519() {
    let (public, private) = make_signing_key_pair();
    let payload = make_payload(public);

    let signature = sign_session_auth_payload(&payload, &private).expect("payload signs");
    assert_eq!(signature.len(), ED25519_SIGNATURE_LENGTH);

    assert!(verify_session_auth_payload(&payload, &signature).expect("payload verifies"));
}

/// `EventLogSessionAuth.test.ts:66` — returns false for mismatched
/// payload/signature.
#[test]
fn returns_false_for_mismatched_payload_and_signature() {
    let (public, private) = make_signing_key_pair();
    let payload = make_payload(public);
    let signature = sign_session_auth_payload(&payload, &private).expect("payload signs");

    let mut tampered = payload.clone();
    tampered.challenge[0] ^= 0xff;

    assert!(!verify_session_auth_payload(&tampered, &signature).expect("payload verifies"));
}

/// `EventLogSessionAuth.test.ts:88` — rejects malformed signature length.
#[test]
fn rejects_malformed_signature_length() {
    let (public, _private) = make_signing_key_pair();
    let payload = make_payload(public);

    let error = verify_session_auth_payload(&payload, &vec![0u8; ED25519_SIGNATURE_LENGTH - 1])
        .expect_err("a short signature is refused");
    assert_eq!(error.reason, SessionAuthErrorReason::InvalidSignatureLength);
    assert_eq!(error.reason.as_str(), "InvalidSignatureLength");
}

/// `EventLogSessionAuth.test.ts:102` — rejects malformed signing public key
/// length at encode time.
#[test]
fn rejects_malformed_signing_public_key_length() {
    let payload = SessionAuthPayload {
        remote_id: RemoteId::Text("remote-123".to_string()),
        challenge: vec![0u8; 32],
        public_key: "client-public-key".to_string(),
        signing_public_key: vec![0u8; ED25519_PUBLIC_KEY_LENGTH - 1],
    };

    let error = encode_session_auth_payload(&payload).expect_err("a short key is refused");
    assert_eq!(
        error.reason,
        SessionAuthErrorReason::InvalidSigningPublicKeyLength
    );
    assert_eq!(error.reason.as_str(), "InvalidSigningPublicKeyLength");
}

/// `EventLogSessionAuth.test.ts:114` — rejects invalid canonical payload bytes.
#[test]
fn rejects_invalid_canonical_payload_bytes() {
    let (public, _private) = make_signing_key_pair();
    let payload = make_payload(public.clone());
    let encoded = encode_session_auth_payload(&payload).expect("payload encodes");
    let malformed = &encoded[..encoded.len() - 1];

    let error =
        verify_session_auth_payload_bytes(malformed, &public, &vec![0u8; ED25519_SIGNATURE_LENGTH])
            .expect_err("a truncated payload is refused");
    assert_eq!(error.reason, SessionAuthErrorReason::InvalidPayload);
    assert_eq!(error.reason.as_str(), "InvalidPayload");
}

/// The keypair helper is load-bearing: a wrong raw public key would make every
/// verify row pass for the wrong reason.
#[test]
fn key_pair_helper_produces_the_public_key_of_its_private_key() {
    let (public, private) = make_signing_key_pair();
    let key = SigningKey::from_pkcs8_der_seed(&private);
    assert_eq!(key.verifying_key().to_bytes().to_vec(), public);
    assert_eq!(
        VerifyingKey::from_bytes(public.as_slice().try_into().unwrap()).unwrap(),
        key.verifying_key()
    );
}

/// The 48-byte PKCS#8 key is `prefix || seed`, so the helper above cannot drift
/// from the shape `sign_session_auth_payload_bytes` parses.
trait Pkcs8Seed {
    fn from_pkcs8_der_seed(key: &[u8]) -> SigningKey;
}

impl Pkcs8Seed for SigningKey {
    fn from_pkcs8_der_seed(key: &[u8]) -> SigningKey {
        assert_eq!(key.len(), ED25519_PKCS8_SEED_PREFIX.len() + 32);
        assert_eq!(
            &key[..ED25519_PKCS8_SEED_PREFIX.len()],
            &ED25519_PKCS8_SEED_PREFIX[..]
        );
        SigningKey::from_bytes(key[ED25519_PKCS8_SEED_PREFIX.len()..].try_into().unwrap())
    }
}
