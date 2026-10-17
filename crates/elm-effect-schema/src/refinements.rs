//! The named refinements the pin spells as schemas: `Char`, `Int`, `Natural`,
//! the four case schemas, and `isUUID`.
//!
//! Each one is a base schema plus a sentence for the refusal. The case schemas
//! are the odd ones: the pin decodes through the transformation and encodes
//! through a check, so decoding normalizes and encoding insists the value was
//! already normalized. That is the rule here too, which is why all four are one
//! shape over four normalize functions.

use crate::{MAX_SAFE_INTEGER, Schema, SchemaError, SchemaKind, Unknown};

/// A refinement's refusal: the sentence, at the value.
fn refusal(expected: &str, value: &Unknown) -> SchemaError {
    SchemaError::TypeMismatchDynamic {
        path: "$".to_string(),
        expected: expected.to_string(),
        got: value.kind(),
    }
}

/// A refinement kind over a base shape, which is how a repr prints it.
fn check_kind(
    base: SchemaKind,
    name: &str,
    args: Vec<String>,
    expected: Option<&str>,
) -> SchemaKind {
    SchemaKind::Check {
        base: Box::new(base),
        name: name.to_string(),
        args,
        annotation: expected
            .map(|expected| vec![("expected".to_string(), expected.to_string())])
            .unwrap_or_default(),
    }
}

impl Schema<String> {
    /// `Schema.Char`: exactly one character.
    pub fn char() -> Self {
        const EXPECTED: &str = "a value with a length of 1";
        Self::new(
            |unknown| match unknown {
                Unknown::Text(text) if text.chars().count() == 1 => Ok(text),
                other => Err(refusal(EXPECTED, &other)),
            },
            |value| {
                if value.chars().count() == 1 {
                    Ok(Unknown::Text(value.clone()))
                } else {
                    Err(refusal(EXPECTED, &Unknown::Text(value.clone())))
                }
            },
            check_kind(
                SchemaKind::String,
                "isLength",
                vec!["1".to_string()],
                Some(EXPECTED),
            ),
        )
    }
}

/// A case schema: decoding normalizes, encoding insists the value was already
/// normalized. "Already normalized" is just "equal to its own normalized form",
/// so the four schemas differ only in the name, the sentence and the function.
fn cased(
    name: &'static str,
    expected: &'static str,
    normalize: fn(&str) -> String,
) -> Schema<String> {
    Schema::<String>::new(
        move |unknown| match unknown {
            Unknown::Text(text) => Ok(normalize(&text)),
            other => Err(refusal(expected, &other)),
        },
        move |value| {
            if *value == normalize(value) {
                Ok(Unknown::Text(value.clone()))
            } else {
                Err(refusal(expected, &Unknown::Text(value.clone())))
            }
        },
        check_kind(SchemaKind::String, name, Vec::new(), Some(expected)),
    )
}

/// Capitalize the first character and leave the rest alone.
fn capitalized(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The same, downward.
fn uncapitalized(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl Schema<String> {
    /// `Schema.Capitalize`: `"abc"` decodes to `"Abc"`, and `"abc"` has no wire
    /// form because it is not already capitalized.
    pub fn capitalize() -> Self {
        cased(
            "isCapitalized",
            "a string with the first character in uppercase",
            capitalized,
        )
    }

    /// `Schema.Uncapitalize`: the mirror.
    pub fn uncapitalize() -> Self {
        cased(
            "isUncapitalized",
            "a string with the first character in lowercase",
            uncapitalized,
        )
    }

    /// `Schema.Lowercase`.
    pub fn lowercase() -> Self {
        cased(
            "isLowercased",
            "a string with all characters in lowercase",
            |value| value.to_lowercase(),
        )
    }

    /// `Schema.Uppercase`.
    pub fn uppercase() -> Self {
        cased(
            "isUppercased",
            "a string with all characters in uppercase",
            |value| value.to_uppercase(),
        )
    }

    /// `Schema.String.check(Schema.isUUID(version))`: the text has to be a UUID,
    /// and with a version the version nibble has to be that one. The unversioned
    /// check admits the nil and max UUIDs, whose version nibble is not one; a
    /// versioned check does not, which is the row that names them.
    pub fn is_uuid(version: Option<u32>) -> Self {
        let expected = match version {
            Some(version) => format!("a UUID v{version}"),
            None => "a UUID".to_string(),
        };
        let decode_expected = expected.clone();
        let encode_expected = expected.clone();
        Self::new(
            move |unknown| match unknown {
                Unknown::Text(text) if uuid_holds(&text, version) => Ok(text),
                other => Err(refusal(&decode_expected, &other)),
            },
            move |value| {
                if uuid_holds(value, version) {
                    Ok(Unknown::Text(value.clone()))
                } else {
                    Err(refusal(&encode_expected, &Unknown::Text(value.clone())))
                }
            },
            check_kind(
                SchemaKind::String,
                "isUUID",
                match version {
                    Some(version) => vec![version.to_string()],
                    None => Vec::new(),
                },
                Some(&expected),
            ),
        )
    }
}

/// Whether the text is a UUID: eight, four, four, four and twelve hexadecimal
/// digits, where the third group's first digit is the version and the fourth
/// group's first is the variant. With no version asked for, the nil and max
/// UUIDs are the two forms that pass without one.
fn uuid_holds(text: &str, version: Option<u32>) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    let widths = [8, 4, 4, 4, 12];
    if groups.len() != widths.len()
        || !groups.iter().zip(widths).all(|(group, width)| {
            group.len() == width && group.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return false;
    }
    if version.is_none() {
        // The two special forms are all-zeros and all-`f`, which is exactly what
        // makes them the UUIDs a versioned check refuses. Hex digits are
        // case-insensitive, so the uppercase max form counts as the same one.
        let digits = text.to_ascii_lowercase().replace('-', "");
        if digits.chars().all(|digit| digit == '0') || digits.chars().all(|digit| digit == 'f') {
            return true;
        }
    }
    let nibble = |group: &str| (group.as_bytes()[0] as char).to_digit(16).unwrap_or(16);
    let version_ok = match version {
        Some(version) => nibble(groups[2]) == version,
        // The unversioned check takes the eight versions that exist.
        None => (1..=8).contains(&nibble(groups[2])),
    };
    version_ok && (8..=11).contains(&nibble(groups[3]))
}

/// An integer in the sense every `isInt` here uses: no fraction, and inside the
/// safe-integer range, so a double past `Number.MAX_SAFE_INTEGER` is refused.
fn is_int(number: f64) -> bool {
    number.fract() == 0.0 && number.abs() <= MAX_SAFE_INTEGER
}

/// A natural: an integer and not negative. The two refusals name themselves, so
/// a fraction is refused for being one and a negative integer for being
/// negative.
fn natural_number(value: &Unknown) -> Result<f64, SchemaError> {
    let number = match value {
        Unknown::Number(number) if is_int(*number) => *number,
        other => return Err(refusal("an integer", other)),
    };
    if number < 0.0 {
        return Err(refusal(
            "a value greater than or equal to 0",
            &Unknown::Number(number),
        ));
    }
    Ok(number)
}

impl Schema<f64> {
    /// `Schema.Int`.
    pub fn int() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Number(number) if is_int(number) => Ok(number),
                other => Err(refusal("an integer", &other)),
            },
            |value| {
                if is_int(*value) {
                    Ok(Unknown::Number(*value))
                } else {
                    Err(refusal("an integer", &Unknown::Number(*value)))
                }
            },
            check_kind(SchemaKind::Number, "isInt", Vec::new(), None),
        )
    }

    /// `Schema.Natural`: `Int` and not negative.
    pub fn natural() -> Self {
        Self::new(
            |unknown| natural_number(&unknown),
            |value| {
                natural_number(&Unknown::Number(*value))?;
                Ok(Unknown::Number(*value))
            },
            check_kind(
                check_kind(SchemaKind::Number, "isInt", Vec::new(), None),
                "isGreaterThanOrEqualTo",
                vec!["0".to_string()],
                None,
            ),
        )
    }
}

/// One runnable check for the rule the UUID rows turn on: a versioned check
/// refuses the nil and max UUIDs that the unversioned one admits.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_special_uuids_pass_only_the_unversioned_check() {
        let nil = "00000000-0000-0000-0000-000000000000";
        let max = "ffffffff-ffff-ffff-ffff-ffffffffffff";
        assert!(uuid_holds(nil, None) && uuid_holds(max, None));
        assert!(!uuid_holds(nil, Some(4)) && !uuid_holds(max, Some(4)));
        assert!(uuid_holds("00000000-0000-4000-8000-000000000001", Some(4)));
        // A plain UUID whose third group starts at zero is neither special form,
        // and its variant nibble is fine, so only the version refuses it.
        assert!(!uuid_holds("00000000-0000-0000-0000-000000000001", None));
        assert!(!uuid_holds("not-a-uuid", None));
    }

    #[test]
    fn the_case_transforms_are_their_own_fixed_points() {
        assert_eq!(capitalized("abc"), "Abc");
        assert_eq!(uncapitalized("Abc"), "abc");
        assert_eq!(capitalized(""), "");
        assert_eq!(uncapitalized(""), "");
    }
}
