---
tier: perishable
tags: [elm-effect, schema-binary, varint, f64, integer-layout, codec]
bead: elm-effect-rs-port-schemabinary-302
created: 2026-10-13
verify: cargo test -p elm-effect-unstable --test schema_binary
verify_result: ok (15 passed; 0 failed)
---
# A sign-magnitude varint built in f64 loses its top bit

## What happened

Carrying SchemaBinary row :487 (the integer layout, which writes bare
sign-magnitude varints and skips the mode byte on an array and the length
prefix on a tuple slot) required a roundtrip over
`[0, -0, 1, -1, 2^48, -2^48, MAX_SAFE_INTEGER, MIN_SAFE_INTEGER]`. After the
encoder and decoder were in place, the largest of those failed with the
decoder's own bound error, `integer varint outside the safe-number range`,
on a value the encoder had just produced.

## The bug

`number_varint` built its word as `(magnitude * 2.0 + sign) as u64`. For
magnitude 2^53 - 1 that product is 2^54 - 1, and f64 spacing in
`[2^53, 2^54)` is 2, so the value rounds up to 2^54. The decoder's
`word >> 1` then reads a magnitude of 2^53, one past `MAX_SAFE_INTEGER`, and
rejects it. Encoder and decoder were exact mirrors; only the arithmetic
between them was not.

The general number path never showed this because it caps magnitudes at
2^48 - 1, where `magnitude * 2 + 1` still has a f64 bit to land on. Only the
integer layout, whose bound is 2^53 - 1, reaches the rounding band.

## The fix

Build the word in integer arithmetic: `uvarint((magnitude as u64) * 2 +
u64::from(negative), out)`. `magnitude as u64` is exact for any integral f64
in range, so the whole word is. This is what the reference does when it
switches from Number to BigInt arithmetic above `EXACT_MAGNITUDE_MAX`
(2^52 - 1).

## Lesson

A two-sided codec can disagree with itself when the encoding arithmetic runs
above the exact-integer range of the type carrying it. The symptom points at
the decoder, and the fix is in the encoder. When a packed form multiplies a
value before packing it, check that the multiply stays inside the exact range
of its carrier, not just inside the range the format allows.
