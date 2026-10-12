//! SchemaBinary wire slice 1: number/boolean array runs.
//!
//! Covers the `0x20` array envelope with the three number-run modes
//! (raw f64, sign-magnitude varint, short decimal) and raw boolean bytes.
//! Structs, rows, string interning, and fingerprint mode come later.

/// Decode failure for this module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryError {
    message: String,
}

impl BinaryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human cause.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for BinaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SchemaBinary: {}", self.message)
    }
}

impl std::error::Error for BinaryError {}

/// Expected envelope description, mirroring the reference codec.
const EXPECTED_ENVELOPE: &str = "version 2 envelope, flags 0";

/// Array-run envelope tag.
const ARRAY_TAG: u8 = 0x20;
/// Number-run element modes.
const NUMBER_RUN_F64: u8 = 0;
const NUMBER_RUN_VARINT: u8 = 1;
const NUMBER_RUN_DECIMAL: u8 = 2;
/// Array-of-arrays run: elements are inner bodies without length or tag.
const NESTED_RUN: u8 = 4;
/// Length-prefixed string run (the reference codec writes no mode byte for
/// strings; the schema picks the decoder).
const STRING_RUN: u8 = 1;
/// Varint path bound (2^48 - 1).
const VARINT_NUMBER_MAX_MAGNITUDE: f64 = 281_474_976_710_655.0;
/// Short-decimal mantissa bound (2^41 - 1).
const DECIMAL_MANTISSA_MAX: f64 = 2_199_023_255_551.0;
/// Short-decimal scale cap.
const DECIMAL_SCALE_MAX: u32 = 8;
const POW10: [f64; 9] = [
    1.0,
    10.0,
    100.0,
    1_000.0,
    10_000.0,
    100_000.0,
    1_000_000.0,
    10_000_000.0,
    100_000_000.0,
];

fn uvarint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn uvarint_decode(input: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *input.get(*pos)?;
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        if shift >= 64 {
            return None;
        }
    }
}

/// JavaScript `Math.round`: halves toward +inf. The only divergence from
/// `(value + 0.5).floor()`, the `-0.5 -> -0` case, can only produce a zero
/// mantissa, which never equals a nonzero value, so plain floor matches
/// scale selection observably.
fn js_round(value: f64) -> f64 {
    if value.fract() == -0.5 {
        -0.0
    } else {
        (value + 0.5).floor()
    }
}

fn is_varint_number(value: f64) -> bool {
    value.fract() == 0.0 && value.abs() <= VARINT_NUMBER_MAX_MAGNITUDE
}

/// Sign-magnitude varint (preserves `-0`, unlike zigzag).
fn number_varint(value: f64, out: &mut Vec<u8>) {
    let negative = value < 0.0 || (value == 0.0 && (1.0 / value) < 0.0);
    let magnitude = if negative { -value } else { value };
    // Integer arithmetic on purpose: the integer layout packs magnitudes up to
    // 2^53 - 1, where `magnitude * 2.0 + 1.0` has no f64 bit to land on and
    // rounds to 2^54, which the decoder then reads as an out-of-range value.
    // `magnitude as u64` is exact for any integral f64, and the reference does
    // the same switch to bigint arithmetic above 2^52 - 1.
    uvarint((magnitude as u64) * 2 + u64::from(negative), out);
}

/// Smallest scale 1..=8 making `value` an exact short-decimal mantissa, else 0.
fn decimal_scale(value: f64) -> u32 {
    for scale in 1..=DECIMAL_SCALE_MAX {
        let mantissa = js_round(value * POW10[scale as usize]);
        if !(mantissa >= -DECIMAL_MANTISSA_MAX && mantissa <= DECIMAL_MANTISSA_MAX) {
            return 0;
        }
        if mantissa / POW10[scale as usize] == value {
            return scale;
        }
    }
    0
}

fn write_decimal_element(value: f64, scale: u32, out: &mut Vec<u8>) {
    let mantissa = if scale == 0 {
        value
    } else {
        js_round(value * POW10[scale as usize])
    };
    let negative = mantissa < 0.0 || (mantissa == 0.0 && (1.0 / mantissa) < 0.0);
    let magnitude = if negative { -mantissa } else { mantissa };
    let word = (magnitude * 2.0 + if negative { 1.0 } else { 0.0 }) * 16.0 + scale as f64;
    uvarint(word as u64, out);
}

fn sign_magnitude_to_f64(word: u64, scale: u32) -> f64 {
    let negative = word & 1 == 1;
    let magnitude = (word >> 1) as f64;
    let value = if scale == 0 {
        magnitude
    } else {
        magnitude / POW10[scale as usize]
    };
    if negative { -value } else { value }
}

/// One tag plus payload behind a length prefix: the shape a scalar frame and
/// a fixed tuple share, since neither writes a count.
fn tagged_frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    uvarint(1 + payload.len() as u64, &mut out);
    out.push(ARRAY_TAG);
    out.extend_from_slice(payload);
    out
}

fn frame(tag: u8, count: u64, mode: Option<u8>, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![tag];
    uvarint(count, &mut body);
    if let Some(mode) = mode {
        body.push(mode);
    }
    body.extend_from_slice(payload);
    let mut out = Vec::new();
    uvarint(body.len() as u64, &mut out);
    out.extend_from_slice(&body);
    out
}

fn hex_preview(bytes: &[u8]) -> String {
    bytes.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Encode `values` as one array run, picking the f64/varint/decimal mode
/// from the elements like the reference codec does.
pub fn encode_number_array(values: &[f64]) -> Vec<u8> {
    let mut varint = true;
    let mut decimal = true;
    let mut scales = vec![0u32; values.len()];
    for (i, value) in values.iter().enumerate() {
        if is_varint_number(*value) {
            if value.abs() > DECIMAL_MANTISSA_MAX {
                decimal = false;
            }
        } else {
            varint = false;
            let scale = decimal_scale(*value);
            if scale == 0 {
                decimal = false;
            } else {
                scales[i] = scale;
            }
        }
        if !varint && !decimal {
            break;
        }
    }
    let mut payload = Vec::new();
    let mode = if varint {
        for value in values {
            number_varint(*value, &mut payload);
        }
        NUMBER_RUN_VARINT
    } else if decimal {
        for (value, scale) in values.iter().zip(scales.iter()) {
            // Integers keep scale 0 even when a scale would fit.
            let scale = if value.fract() == 0.0 { 0 } else { *scale };
            write_decimal_element(*value, scale, &mut payload);
        }
        NUMBER_RUN_DECIMAL
    } else {
        for value in values {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        NUMBER_RUN_F64
    };
    frame(ARRAY_TAG, values.len() as u64, Some(mode), &payload)
}

/// Largest exactly-representable integer magnitude (2^53 - 1).
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// The guard the integer layout applies to every value it packs. The
/// reference reaches this check even when the layout was chosen from another
/// schema (`disableChecks`), so the encoder cannot assume its input is
/// integral; `Number.isSafeInteger` is finiteness plus the 2^53 - 1 bound.
fn int_element(value: f64, index: Option<usize>) -> Result<f64, BinaryError> {
    if value.fract() != 0.0 || !value.is_finite() || value.abs() > MAX_SAFE_INTEGER as f64 {
        let at = match index {
            Some(index) => format!(" at [{index}]"),
            None => String::new(),
        };
        return Err(BinaryError::new(format!(
            "Expected an integer, got {value}{at}"
        )));
    }
    Ok(value)
}

/// Encode one scalar in the integer layout: a bare sign-magnitude varint, no
/// length prefix on the slot and no mode byte.
pub fn encode_int_scalar(value: f64) -> Result<Vec<u8>, BinaryError> {
    let value = int_element(value, None)?;
    let mut payload = Vec::new();
    number_varint(value, &mut payload);
    Ok(tagged_frame(&payload))
}

/// Encode an integer array: the count, then bare varints. The schema proves
/// the element layout, so no mode byte is written.
pub fn encode_int_array(values: &[f64]) -> Result<Vec<u8>, BinaryError> {
    let mut payload = Vec::new();
    for (index, value) in values.iter().enumerate() {
        number_varint(int_element(*value, Some(index))?, &mut payload);
    }
    Ok(frame(ARRAY_TAG, values.len() as u64, None, &payload))
}

/// Encode a fixed tuple of integers: inline slots and no count, since the
/// slot count is in the schema.
pub fn encode_int_tuple(values: &[f64]) -> Result<Vec<u8>, BinaryError> {
    let mut payload = Vec::new();
    for (index, value) in values.iter().enumerate() {
        number_varint(int_element(*value, Some(index))?, &mut payload);
    }
    Ok(tagged_frame(&payload))
}

/// Encode a fixed tuple of numbers: each slot is a length-prefixed payload,
/// so a wide f64 slot stays self-delimiting without a second tag.
pub fn encode_number_tuple(values: &[f64]) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in values {
        let slot = number_scalar_payload(*value);
        uvarint(slot.len() as u64, &mut payload);
        payload.extend_from_slice(&slot);
    }
    tagged_frame(&payload)
}

/// Decode one integer array: count then bare varints, with the same
/// safe-number bound the scalar decoder enforces.
pub fn decode_int_array(bytes: &[u8]) -> Result<Vec<f64>, BinaryError> {
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let mut pos = 0;
    let _length = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated length"))?;
    let tag = *bytes.get(pos).ok_or_else(|| fail("truncated tag"))?;
    pos += 1;
    if tag != ARRAY_TAG {
        return Err(fail(&format!(
            "expected {EXPECTED_ENVELOPE}, actual {tag:#04x}"
        )));
    }
    let count = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated count"))?;
    let mut out = Vec::new();
    for _ in 0..count {
        let word = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("Expected complete value"))?;
        let negative = word & 1 == 1;
        let magnitude = word >> 1;
        if magnitude > MAX_SAFE_INTEGER {
            return Err(fail("integer varint outside the safe-number range"));
        }
        let value = magnitude as f64;
        out.push(if negative { -value } else { value });
    }
    if pos != bytes.len() {
        return Err(fail("no leftover bytes"));
    }
    Ok(out)
}

/// Encode a uniform number array from the pin's untyped side: every element
/// must be a number, and a failure names the index that was not.
pub fn encode_number_array_unknown(values: &[serde_json::Value]) -> Result<Vec<u8>, BinaryError> {
    let mut numbers = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        match value.as_f64() {
            Some(number) => numbers.push(number),
            None => {
                return Err(BinaryError::new(format!(
                    "Expected a number, got {value} at [{index}]"
                )));
            }
        }
    }
    Ok(encode_number_array(&numbers))
}

/// Decode one scalar integer: bare sign-magnitude varint only, magnitudes
/// past the safe range fail instead of rounding.
pub fn decode_int_scalar(bytes: &[u8]) -> Result<f64, BinaryError> {
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let mut pos = 0;
    let _length = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated length"))?;
    let tag = *bytes.get(pos).ok_or_else(|| fail("truncated tag"))?;
    pos += 1;
    if tag != ARRAY_TAG {
        return Err(fail(&format!(
            "expected {EXPECTED_ENVELOPE}, actual {tag:#04x}"
        )));
    }
    let word = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated scalar"))?;
    if pos != bytes.len() {
        return Err(fail("no leftover bytes"));
    }
    let negative = word & 1 == 1;
    let magnitude = word >> 1;
    if magnitude > MAX_SAFE_INTEGER {
        return Err(fail("integer varint outside the safe-number range"));
    }
    let value = magnitude as f64;
    Ok(if negative { -value } else { value })
}

/// Encode one scalar number: bare sign-magnitude varint, mantissa
/// varint plus scale byte for short decimals, or raw f64.
/// Scalar number payload without the envelope: bare varint,
/// mantissa varint plus scale byte, or raw f64.
fn number_scalar_payload(value: f64) -> Vec<u8> {
    let mut payload = Vec::new();
    if is_varint_number(value) {
        number_varint(value, &mut payload);
    } else {
        let scale = decimal_scale(value);
        if scale == 0 {
            payload.extend_from_slice(&value.to_le_bytes());
        } else {
            number_varint(js_round(value * POW10[scale as usize]), &mut payload);
            payload.push(scale as u8);
        }
    }
    payload
}

/// Encode one scalar number: bare sign-magnitude varint, mantissa
/// varint plus scale byte for short decimals, or raw f64.
pub fn encode_number_scalar(value: f64) -> Vec<u8> {
    tagged_frame(&number_scalar_payload(value))
}

/// Union value: the two forms row 457 covers.
#[derive(Clone, Debug, PartialEq)]
pub enum UnionValue {
    Number(f64),
    Text(String),
}

/// Union member tags.
const UNION_NUMBER: u8 = 4;
const UNION_STRING: u8 = 5;

/// Encode one union member: kind byte plus the scalar number payload or
/// raw string bytes.
pub fn encode_union(value: &UnionValue) -> Vec<u8> {
    let mut payload = Vec::new();
    match value {
        UnionValue::Number(value) => {
            payload.push(UNION_NUMBER);
            payload.extend_from_slice(&number_scalar_payload(*value));
        }
        UnionValue::Text(text) => {
            payload.push(UNION_STRING);
            payload.extend_from_slice(text.as_bytes());
        }
    }
    tagged_frame(&payload)
}

/// Decode one union member.
pub fn decode_union(bytes: &[u8]) -> Result<UnionValue, BinaryError> {
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let mut pos = 0;
    let _length = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated length"))?;
    let tag = *bytes.get(pos).ok_or_else(|| fail("truncated tag"))?;
    pos += 1;
    if tag != ARRAY_TAG {
        return Err(fail(&format!(
            "expected {EXPECTED_ENVELOPE}, actual {tag:#04x}"
        )));
    }
    let kind = *bytes.get(pos).ok_or_else(|| fail("truncated kind"))?;
    pos += 1;
    if kind == UNION_STRING {
        let text = std::str::from_utf8(&bytes[pos..]).map_err(|_| fail("invalid utf-8"))?;
        if pos + text.len() != bytes.len() {
            return Err(fail("no leftover bytes"));
        }
        return Ok(UnionValue::Text(text.to_owned()));
    }
    if kind != UNION_NUMBER {
        return Err(fail("unknown union kind"));
    }
    if bytes.len() - pos == 8 {
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[pos..pos + 8]);
        return Ok(UnionValue::Number(f64::from_le_bytes(word)));
    }
    let word = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated union number"))?;
    if pos == bytes.len() {
        return Ok(UnionValue::Number(sign_magnitude_to_f64(word, 0)));
    }
    if pos + 1 != bytes.len() {
        return Err(fail("no leftover bytes"));
    }
    let scale = bytes[pos];
    pos += 1;
    if scale > DECIMAL_SCALE_MAX as u8 {
        return Err(fail("bad union scale"));
    }
    let negative = word & 1 == 1;
    let magnitude = (word >> 1) as f64;
    let value = magnitude / POW10[scale as usize];
    Ok(UnionValue::Number(if negative { -value } else { value }))
}

/// Split concatenated length-prefixed frames (tuple slots).
pub fn split_frames(bytes: &[u8]) -> Result<Vec<&[u8]>, BinaryError> {
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < bytes.len() {
        let start = pos;
        let len =
            uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated frame length"))? as usize;
        let end = pos
            .checked_add(len)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| fail("truncated frame"))?;
        out.push(&bytes[start..end]);
        pos = end;
    }
    Ok(out)
}

/// Payload extent bounds: eight bytes is the f64 form, and a sign-magnitude
/// varint never needs more than seven.
const NUMBER_VARINT_MAX_BYTES: usize = 7;

/// Parse the non-f64 scalar payload forms with the reference's vocabulary: a
/// varint cut short is "Expected complete value", and anything else that is
/// not exactly one in-range nonzero scale byte is "Expected decimal" (scale
/// 0 is illegal here because the bare varint is that value's canonical form).
fn decode_scalar_payload(payload: &[u8]) -> Result<f64, &'static str> {
    let mut pos = 0;
    let word = uvarint_decode(payload, &mut pos).ok_or("Expected complete value")?;
    if pos == payload.len() {
        return Ok(sign_magnitude_to_f64(word, 0));
    }
    if pos + 1 != payload.len() {
        return Err("Expected decimal");
    }
    let scale = payload[pos];
    if scale == 0 || scale > DECIMAL_SCALE_MAX as u8 {
        return Err("Expected decimal");
    }
    // The mantissa word of a scalar decimal reuses the sign-magnitude form.
    let negative = word & 1 == 1;
    let magnitude = (word >> 1) as f64;
    let value = magnitude / POW10[scale as usize];
    Ok(if negative { -value } else { value })
}

/// Decode one scalar number: 8-byte payloads are f64, otherwise a
/// varint with an optional trailing scale byte. The frame's own length
/// bounds the payload, so `trailing bytes outside the frame` and `trailing
/// bytes inside it` stay distinguishable.
pub fn decode_number_scalar(bytes: &[u8]) -> Result<f64, BinaryError> {
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let mut pos = 0;
    let length = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated length"))? as usize;
    let end = pos
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| fail("truncated frame"))?;
    let tag = *bytes.get(pos).ok_or_else(|| fail("truncated tag"))?;
    pos += 1;
    if tag != ARRAY_TAG {
        return Err(fail(&format!(
            "expected {EXPECTED_ENVELOPE}, actual {tag:#04x}"
        )));
    }
    let payload = &bytes[pos..end];
    let value = if payload.len() == 8 {
        let mut word = [0u8; 8];
        word.copy_from_slice(payload);
        f64::from_le_bytes(word)
    } else if payload.is_empty() || payload.len() > NUMBER_VARINT_MAX_BYTES {
        return Err(fail("Expected f64"));
    } else {
        decode_scalar_payload(payload).map_err(&fail)?
    };
    if end != bytes.len() {
        return Err(fail("no leftover bytes"));
    }
    Ok(value)
}

/// Encode strings as length-prefixed UTF-8 behind the array envelope.
pub fn encode_string_array(values: &[&str]) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in values {
        uvarint(value.len() as u64, &mut payload);
        payload.extend_from_slice(value.as_bytes());
    }
    frame(ARRAY_TAG, values.len() as u64, None, &payload)
}

/// Decode one length-prefixed string run (no mode byte on the wire).
pub fn decode_string_array(bytes: &[u8]) -> Result<Vec<String>, BinaryError> {
    let mut pos = 0;
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let _length = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated length"))?;
    let tag = *bytes.get(pos).ok_or_else(|| fail("truncated tag"))?;
    pos += 1;
    if tag != ARRAY_TAG {
        return Err(fail(&format!(
            "expected {EXPECTED_ENVELOPE}, actual {tag:#04x}"
        )));
    }
    let count = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated count"))?;
    let _ = STRING_RUN;
    let mut out = Vec::new();
    for _ in 0..count {
        let len = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated string length"))?
            as usize;
        let end = pos
            .checked_add(len)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| fail("truncated string bytes"))?;
        let text = std::str::from_utf8(&bytes[pos..end]).map_err(|_| fail("invalid utf-8"))?;
        out.push(text.to_owned());
        pos = end;
    }
    if pos != bytes.len() {
        return Err(fail("no leftover bytes"));
    }
    Ok(out)
}

/// Encode booleans as raw bytes behind the array envelope (no run mode).
pub fn encode_bool_array(values: &[bool]) -> Vec<u8> {
    let payload: Vec<u8> = values.iter().map(|value| u8::from(*value)).collect();
    frame(ARRAY_TAG, values.len() as u64, None, &payload)
}

struct Cursor<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn fail(&self, message: &str) -> BinaryError {
        BinaryError::new(format!(
            "{} [{}]",
            message,
            hex_preview(&self.input[self.pos..])
        ))
    }

    fn uvarint(&mut self) -> Result<u64, BinaryError> {
        uvarint_decode(self.input, &mut self.pos).ok_or_else(|| self.fail("truncated varint"))
    }

    fn byte(&mut self) -> Result<u8, BinaryError> {
        let byte = *self
            .input
            .get(self.pos)
            .ok_or_else(|| self.fail("truncated body"))?;
        self.pos += 1;
        Ok(byte)
    }
}

fn read_envelope(input: &[u8], pos: &mut usize) -> Result<(u64, u8), BinaryError> {
    let mut cursor = Cursor { input, pos: *pos };
    let _length = cursor.uvarint()?;
    let tag = cursor.byte()?;
    if tag != ARRAY_TAG {
        return Err(cursor.fail(&format!("expected {EXPECTED_ENVELOPE}, actual {tag:#04x}")));
    }
    let count = cursor.uvarint()?;
    let mode = cursor.byte()?;
    *pos = cursor.pos;
    Ok((count, mode))
}

/// Decode one number run body shared by flat and nested arrays.
fn decode_number_run(
    bytes: &[u8],
    mut pos: &mut usize,
    count: u64,
    mode: u8,
) -> Result<Vec<f64>, BinaryError> {
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let mut out = Vec::new();
    match mode {
        NUMBER_RUN_F64 => {
            let need = count.checked_mul(8).ok_or_else(|| fail("count overflow"))?;
            if bytes.len() - *pos < need as usize {
                return Err(fail("Expected f64"));
            }
            for _ in 0..count {
                let mut word = [0u8; 8];
                word.copy_from_slice(&bytes[*pos..*pos + 8]);
                *pos += 8;
                out.push(f64::from_le_bytes(word));
            }
        }
        NUMBER_RUN_VARINT => {
            for _ in 0..count {
                let word = uvarint_decode(bytes, &mut pos)
                    .ok_or_else(|| fail("Expected complete value"))?;
                out.push(sign_magnitude_to_f64(word, 0));
            }
        }
        NUMBER_RUN_DECIMAL => {
            for _ in 0..count {
                let word = uvarint_decode(bytes, &mut pos)
                    .ok_or_else(|| fail("Expected complete value"))?;
                let scale = (word % 16) as u32;
                // Scale 0 is legal in a run: it is the integer form. Only a
                // scale past the decimal cap is a malformed element.
                if scale > DECIMAL_SCALE_MAX {
                    return Err(fail("Expected decimal"));
                }
                out.push(sign_magnitude_to_f64(word / 16, scale));
            }
        }
        _ => return Err(fail("Expected f64")),
    }
    Ok(out)
}

/// Decode one number array run.
pub fn decode_number_array(bytes: &[u8]) -> Result<Vec<f64>, BinaryError> {
    let mut pos = 0;
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let (count, mode) = read_envelope(bytes, &mut pos)?;
    let out = decode_number_run(bytes, &mut pos, count, mode)?;
    if pos != bytes.len() {
        return Err(fail("no leftover bytes"));
    }
    Ok(out)
}

/// Encode nested number arrays: inner bodies without length or tag.
pub fn encode_nested_number_array(values: &[Vec<f64>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for inner in values {
        let frame = encode_number_array(inner);
        payload.extend_from_slice(&frame[2..]);
    }
    frame(ARRAY_TAG, values.len() as u64, Some(NESTED_RUN), &payload)
}

/// Decode one nested number array run.
pub fn decode_nested_number_array(bytes: &[u8]) -> Result<Vec<Vec<f64>>, BinaryError> {
    let mut pos = 0;
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let (count, mode) = read_envelope(bytes, &mut pos)?;
    if mode != NESTED_RUN {
        return Err(fail("not a nested run"));
    }
    let mut out = Vec::new();
    for _ in 0..count {
        let mut cursor = Cursor { input: bytes, pos };
        let inner_count = cursor
            .uvarint()
            .map_err(|_| fail("truncated nested count"))?;
        let inner_mode = cursor.byte().map_err(|_| fail("truncated nested mode"))?;
        pos = cursor.pos;
        out.push(decode_number_run(bytes, &mut pos, inner_count, inner_mode)?);
    }
    if pos != bytes.len() {
        return Err(fail("no leftover bytes"));
    }
    Ok(out)
}

/// Decode one boolean array run.
pub fn decode_bool_array(bytes: &[u8]) -> Result<Vec<bool>, BinaryError> {
    let mut pos = 0;
    let fail = |message: &str| BinaryError::new(format!("{} [{}]", message, hex_preview(bytes)));
    let _length = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated length"))?;
    let tag = *bytes.get(pos).ok_or_else(|| fail("truncated tag"))?;
    pos += 1;
    if tag != ARRAY_TAG {
        return Err(fail(&format!(
            "expected {EXPECTED_ENVELOPE}, actual {tag:#04x}"
        )));
    }
    let count = uvarint_decode(bytes, &mut pos).ok_or_else(|| fail("truncated count"))?;
    let mut out = Vec::new();
    for _ in 0..count {
        let byte = *bytes.get(pos).ok_or_else(|| fail("truncated bool run"))?;
        pos += 1;
        out.push(byte != 0);
    }
    if pos != bytes.len() {
        return Err(fail("no leftover bytes"));
    }
    Ok(out)
}
