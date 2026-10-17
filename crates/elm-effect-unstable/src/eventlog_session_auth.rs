//! Session auth (`eventlog/EventLogSessionAuth`): the canonical payload a peer
//! signs with Ed25519 so another peer can authenticate a remote session.
//!
//! The payload is a fixed field order, each field framed by a 4-byte big-endian
//! length, so the signed bytes are deterministic for one payload (a JSON
//! encoding would not be). The context field is part of the signed bytes, which
//! is what stops a signature over some other protocol's payload from verifying
//! here.

use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// Domain separation: the first framed field, and the check that a payload came
/// from this protocol.
pub const AUTH_PAYLOAD_CONTEXT: &str = "eventlog-auth-v1";

pub const ED25519_PUBLIC_KEY_LENGTH: usize = 32;
pub const ED25519_SIGNATURE_LENGTH: usize = 64;
pub const SESSION_AUTH_CHALLENGE_LENGTH: usize = 32;
pub const SESSION_AUTH_CHALLENGE_TIME_TO_LIVE_MILLIS: u64 = 30_000;

const LENGTH_PREFIX_BYTES: usize = 4;

/// The pin's `EventLogSessionAuthError.reason` union, narrowed to the reasons
/// this surface can actually produce: Rust has no `WebCrypto` to be
/// unavailable, so `CryptoUnavailable`/`CryptoFailure` have no source here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAuthErrorReason {
    InvalidPayload,
    InvalidContext,
    InvalidSigningPublicKeyLength,
    InvalidSignatureLength,
    InvalidSigningPrivateKey,
}

impl SessionAuthErrorReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidPayload => "InvalidPayload",
            Self::InvalidContext => "InvalidContext",
            Self::InvalidSigningPublicKeyLength => "InvalidSigningPublicKeyLength",
            Self::InvalidSignatureLength => "InvalidSignatureLength",
            Self::InvalidSigningPrivateKey => "InvalidSigningPrivateKey",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAuthError {
    pub reason: SessionAuthErrorReason,
    pub message: String,
}

impl SessionAuthError {
    fn new(reason: SessionAuthErrorReason, message: String) -> Self {
        Self { reason, message }
    }
}

/// A remote id is a name, or opaque bytes kept as lowercase hex inside the
/// signed bytes (`bytesToHex`), because a field is frames of bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteId {
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAuthPayload {
    pub remote_id: RemoteId,
    pub challenge: Vec<u8>,
    pub public_key: String,
    pub signing_public_key: Vec<u8>,
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn assert_signing_public_key_length(key: &[u8]) -> Result<(), SessionAuthError> {
    if key.len() == ED25519_PUBLIC_KEY_LENGTH {
        return Ok(());
    }
    Err(SessionAuthError::new(
        SessionAuthErrorReason::InvalidSigningPublicKeyLength,
        format!(
            "Expected signingPublicKey length to be {ED25519_PUBLIC_KEY_LENGTH} bytes, received {}",
            key.len()
        ),
    ))
}

fn assert_signature_length(signature: &[u8]) -> Result<(), SessionAuthError> {
    if signature.len() == ED25519_SIGNATURE_LENGTH {
        return Ok(());
    }
    Err(SessionAuthError::new(
        SessionAuthErrorReason::InvalidSignatureLength,
        format!(
            "Expected signature length to be {ED25519_SIGNATURE_LENGTH} bytes, received {}",
            signature.len()
        ),
    ))
}

/// Write one 4-byte big-endian length. A field longer than `u32::MAX` cannot be
/// framed; the pin fails the same way rather than truncating.
fn write_length(out: &mut Vec<u8>, length: usize) -> Result<(), SessionAuthError> {
    let Ok(length) = u32::try_from(length) else {
        return Err(SessionAuthError::new(
            SessionAuthErrorReason::InvalidPayload,
            format!("Invalid canonical field length: {length}"),
        ));
    };
    out.extend_from_slice(&length.to_be_bytes());
    Ok(())
}

/// Read one framed field, advancing `offset` past it.
fn read_field(payload: &[u8], offset: &mut usize) -> Result<Vec<u8>, SessionAuthError> {
    if *offset + LENGTH_PREFIX_BYTES > payload.len() {
        return Err(SessionAuthError::new(
            SessionAuthErrorReason::InvalidPayload,
            "Session auth payload is truncated before field length".to_string(),
        ));
    }
    let length = u32::from_be_bytes([
        payload[*offset],
        payload[*offset + 1],
        payload[*offset + 2],
        payload[*offset + 3],
    ]) as usize;
    *offset += LENGTH_PREFIX_BYTES;
    if *offset + length > payload.len() {
        return Err(SessionAuthError::new(
            SessionAuthErrorReason::InvalidPayload,
            "Session auth payload is truncated before field end".to_string(),
        ));
    }
    let field = payload[*offset..*offset + length].to_vec();
    *offset += length;
    Ok(field)
}

fn decode_utf8(bytes: &[u8]) -> Result<String, SessionAuthError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| {
        SessionAuthError::new(
            SessionAuthErrorReason::InvalidPayload,
            "Session auth payload contains invalid UTF-8 bytes".to_string(),
        )
    })
}

pub fn encode_session_auth_payload(payload: &SessionAuthPayload) -> Result<Vec<u8>, SessionAuthError> {
    assert_signing_public_key_length(&payload.signing_public_key)?;

    let remote_id = match &payload.remote_id {
        RemoteId::Text(text) => text.as_bytes().to_vec(),
        RemoteId::Bytes(bytes) => bytes_to_hex(bytes).into_bytes(),
    };
    let fields: [&[u8]; 5] = [
        AUTH_PAYLOAD_CONTEXT.as_bytes(),
        &remote_id,
        &payload.challenge,
        payload.public_key.as_bytes(),
        &payload.signing_public_key,
    ];

    let mut encoded = Vec::with_capacity(
        fields.iter().map(|field| LENGTH_PREFIX_BYTES + field.len()).sum(),
    );
    for field in fields {
        write_length(&mut encoded, field.len())?;
        encoded.extend_from_slice(field);
    }
    Ok(encoded)
}

/// The decoded `remote_id` is always `Text`: a byte id was hex-encoded on the
/// way in, so it arrives as a name.
pub fn decode_session_auth_payload(payload: &[u8]) -> Result<SessionAuthPayload, SessionAuthError> {
    let mut offset = 0usize;
    let context = decode_utf8(&read_field(payload, &mut offset)?)?;
    if context != AUTH_PAYLOAD_CONTEXT {
        return Err(SessionAuthError::new(
            SessionAuthErrorReason::InvalidContext,
            format!("Invalid session auth payload context: {context}"),
        ));
    }
    let remote_id = decode_utf8(&read_field(payload, &mut offset)?)?;
    let challenge = read_field(payload, &mut offset)?;
    let public_key = decode_utf8(&read_field(payload, &mut offset)?)?;
    let signing_public_key = read_field(payload, &mut offset)?;
    assert_signing_public_key_length(&signing_public_key)?;

    if offset != payload.len() {
        return Err(SessionAuthError::new(
            SessionAuthErrorReason::InvalidPayload,
            "Session auth payload contains trailing bytes".to_string(),
        ));
    }

    Ok(SessionAuthPayload {
        remote_id: RemoteId::Text(remote_id),
        challenge,
        public_key,
        signing_public_key,
    })
}

/// A PKCS#8 `Ed25519` private key is the only shape the pin imports, so an
/// unparseable key is `InvalidSigningPrivateKey` and not a silent seed read.
pub fn sign_session_auth_payload_bytes(
    payload: &[u8],
    signing_private_key: &[u8],
) -> Result<Vec<u8>, SessionAuthError> {
    decode_session_auth_payload(payload)?;

    let signing_key = SigningKey::from_pkcs8_der(signing_private_key).map_err(|_| {
        SessionAuthError::new(
            SessionAuthErrorReason::InvalidSigningPrivateKey,
            "Failed to import Ed25519 signing private key (expected PKCS#8 bytes)".to_string(),
        )
    })?;

    Ok(signing_key.sign(payload).to_bytes().to_vec())
}

/// `false` is a real answer (a well-formed signature that does not match); only
/// a malformed length or payload is an error, which is how the pin splits the
/// two.
pub fn verify_session_auth_payload_bytes(
    payload: &[u8],
    signing_public_key: &[u8],
    signature: &[u8],
) -> Result<bool, SessionAuthError> {
    decode_session_auth_payload(payload)?;
    assert_signing_public_key_length(signing_public_key)?;
    assert_signature_length(signature)?;

    let key_bytes: [u8; ED25519_PUBLIC_KEY_LENGTH] = signing_public_key
        .try_into()
        .expect("length asserted above");
    let verifying_key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| {
        SessionAuthError::new(
            SessionAuthErrorReason::InvalidSigningPublicKeyLength,
            "Failed to import Ed25519 signing public key".to_string(),
        )
    })?;
    let signature_bytes: [u8; ED25519_SIGNATURE_LENGTH] =
        signature.try_into().expect("length asserted above");

    Ok(verifying_key
        .verify(payload, &Signature::from_bytes(&signature_bytes))
        .is_ok())
}

pub fn sign_session_auth_payload(
    payload: &SessionAuthPayload,
    signing_private_key: &[u8],
) -> Result<Vec<u8>, SessionAuthError> {
    let encoded = encode_session_auth_payload(payload)?;
    sign_session_auth_payload_bytes(&encoded, signing_private_key)
}

pub fn verify_session_auth_payload(
    payload: &SessionAuthPayload,
    signature: &[u8],
) -> Result<bool, SessionAuthError> {
    let encoded = encode_session_auth_payload(payload)?;
    verify_session_auth_payload_bytes(&encoded, &payload.signing_public_key, signature)
}
