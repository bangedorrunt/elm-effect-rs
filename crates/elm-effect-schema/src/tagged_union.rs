//! `Schema.toTaggedUnion` and `Schema.TaggedUnion`: a union indexed by the
//! literal its members carry at one key.
//!
//! The walk is the pin's: a union member that is itself a union is descended
//! into, every other member contributes the literal at `tag`, and two members
//! that name the same discriminant are refused. The index is keyed the way the
//! pin keys a JavaScript object: a number literal and its printed spelling are
//! the same key, which is why `1` and `"1"` collide.
//!
//! One thing the pin carries and this port cannot: `guards` is a
//! *decoded-type* reflection (`SchemaParser.is(toType(member))`), and every
//! decoded value here is the same struct type, so reflection has nothing to
//! distinguish. [`TaggedUnion::accepts`] is the useful analogue, and it asks
//! the wire question instead (would this case decode the value), which differs
//! exactly where a member's field transforms its wire form.

use crate::{Field, Schema, SchemaKind, SentinelKey, Unknown, js_number_text, member_sentinels};

/// Why a union could not be indexed by its tag.
#[derive(Debug, Clone, PartialEq)]
pub enum TaggedUnionError {
    /// Two members name the same discriminant. The literal is the one that
    /// arrived second, as the pin's message names it.
    Duplicate(Unknown),
    /// A member carries no literal at the tag key, so it cannot be a case.
    NoDiscriminant,
}

impl std::fmt::Display for TaggedUnionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaggedUnionError::Duplicate(literal) => {
                write!(f, "Duplicate discriminant: {}", printed(literal))
            }
            TaggedUnionError::NoDiscriminant => write!(f, "No literal or unique symbol found"),
        }
    }
}

impl std::error::Error for TaggedUnionError {}

/// A union indexed by the literal each member carries at `tag`
/// (`Schema.toTaggedUnion`).
#[derive(Clone)]
pub struct TaggedUnion {
    /// The key every member discriminates on.
    pub tag: String,
    /// The union itself, codec unchanged: the pin hands the same schema back
    /// with the index attached.
    pub schema: Schema<Unknown>,
    /// The literals the members declare, in walk order.
    pub discriminants: Vec<Unknown>,
    /// Literal and member, in walk order. Look one up with
    /// [`case_for`](Self::case_for), which keys numbers by their printed form.
    pub cases: Vec<(Unknown, Schema<Unknown>)>,
}

/// The discriminant a literal spells as an object key (`String(literal)` for a
/// number, the text itself for a string), or `None` for a value that is not a
/// property key at all.
fn discriminant_key(literal: &Unknown) -> Option<String> {
    match literal {
        Unknown::Text(text) => Some(text.clone()),
        Unknown::Number(number) => Some(js_number_text(*number)),
        _ => None,
    }
}

/// How the pin's message prints a literal (`String(literal)`).
fn printed(literal: &Unknown) -> String {
    discriminant_key(literal).unwrap_or_else(|| literal.kind().to_string())
}

/// Index `union` by the literal its members carry at `tag`.
pub fn to_tagged_union(union: Schema<Unknown>, tag: &str) -> Result<TaggedUnion, TaggedUnionError> {
    let mut discriminants = Vec::new();
    let mut cases = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    walk(&union, tag, &mut seen, &mut discriminants, &mut cases)?;
    Ok(TaggedUnion {
        tag: tag.to_string(),
        schema: union,
        discriminants,
        cases,
    })
}

fn walk(
    schema: &Schema<Unknown>,
    tag: &str,
    seen: &mut Vec<String>,
    discriminants: &mut Vec<Unknown>,
    cases: &mut Vec<(Unknown, Schema<Unknown>)>,
) -> Result<(), TaggedUnionError> {
    // An empty union is `never` in this port (`union_with_mode` resolves the
    // two to one value), so a shape that admits nothing indexes to nothing.
    // The pin keeps a zero-member union node instead, so its `Union([])` walks
    // to an empty index while its `Schema.Never` throws; here both answer with
    // an empty index. That is the one call where the two designs disagree.
    if matches!(schema.repr_kind(), SchemaKind::Opaque("never")) {
        return Ok(());
    }
    // A union keeps its member list only when it was built from one; a
    // two-member `union` falls through to the sentinel path below, which is
    // the pin's behaviour for a `members`-less union too. `Literals` answers a
    // member list of its own, and descending into it would refuse every case.
    let is_union = matches!(schema.repr_kind(), SchemaKind::Union { .. });
    if is_union {
        if let Some(members) = schema.members() {
            for member in &members {
                walk(member, tag, seen, discriminants, cases)?;
            }
            return Ok(());
        }
    }
    let literal = member_sentinels(schema)
        .into_iter()
        .find_map(|(key, value)| match key {
            SentinelKey::Field(name) if name == tag => Some(value),
            _ => None,
        });
    let Some(literal) = literal else {
        return Err(TaggedUnionError::NoDiscriminant);
    };
    let Some(key) = discriminant_key(&literal) else {
        return Err(TaggedUnionError::NoDiscriminant);
    };
    if seen.contains(&key) {
        return Err(TaggedUnionError::Duplicate(literal));
    }
    seen.push(key);
    discriminants.push(literal.clone());
    cases.push((literal, schema.clone()));
    Ok(())
}

impl TaggedUnion {
    /// The case whose discriminant keys to `discriminant`. A number and its
    /// printed spelling are the same key, as they are in the pin's object.
    pub fn case_for(&self, discriminant: &Unknown) -> Option<&Schema<Unknown>> {
        let wanted = discriminant_key(discriminant)?;
        self.cases
            .iter()
            .find(|(literal, _)| discriminant_key(literal).as_deref() == Some(wanted.as_str()))
            .map(|(_, schema)| schema)
    }

    /// The discriminant the value carries at this union's tag, keyed.
    pub fn tag_of(&self, value: &Unknown) -> Option<String> {
        match value {
            Unknown::Object(map) => discriminant_key(map.get(&self.tag)?),
            _ => None,
        }
    }

    /// Whether the value carries one of `keys` at this union's tag
    /// (`isAnyOf`).
    pub fn is_any_of(&self, keys: &[Unknown], value: &Unknown) -> bool {
        let Some(found) = self.tag_of(value) else {
            return false;
        };
        keys.iter()
            .filter_map(discriminant_key)
            .any(|key| key == found)
    }

    /// Whether this case would decode the value. The pin spells this `guards`
    /// and reflects the *decoded* type; see the module note.
    pub fn accepts(&self, discriminant: &Unknown, value: &Unknown) -> bool {
        let Some(case) = self.case_for(discriminant) else {
            return false;
        };
        case.decode(value.clone()).is_ok()
    }

    /// `matchOrElse`: the handler registered for the value's tag, else the
    /// fallback. A case present with no handler (`None`, the pin's
    /// `undefined`) also runs the fallback, which is `matchOrElse`'s rule.
    pub fn dispatch<R: Clone>(
        &self,
        value: &Unknown,
        cases: &[(Unknown, Option<R>)],
        or_else: Option<&R>,
    ) -> Option<R> {
        let found = self.tag_of(value);
        if let Some(found) = found {
            for (literal, handler) in cases {
                if discriminant_key(literal).as_deref() == Some(found.as_str()) {
                    if let Some(handler) = handler {
                        return Some(handler.clone());
                    }
                    break;
                }
            }
        }
        or_else.cloned()
    }
}

/// `Schema.TaggedUnion`: build the union from a case list, then index it by
/// `_tag`. Each case key becomes the member's `_tag` literal.
pub fn tagged_union(cases: Vec<(&str, Vec<Field>)>) -> Result<TaggedUnion, TaggedUnionError> {
    let members = cases
        .into_iter()
        .map(|(name, fields)| {
            let mut declared = vec![Field::tag("_tag", Unknown::Text(name.to_string()))];
            declared.extend(fields);
            Schema::as_unknown(
                Schema::<std::collections::BTreeMap<String, Unknown>>::struct_shape(declared),
            )
        })
        .collect();
    let union = Schema::<Unknown>::union_with_mode(members, "anyOf");
    to_tagged_union(union, "_tag")
}

/// One runnable check for the two rules that are easy to get wrong: the index
/// keys a number by its printed spelling, and a duplicate is caught across a
/// nested union rather than only within one member list.
#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str, value: &str) -> Field {
        Field::tag(name, Unknown::Text(value.to_string()))
    }

    fn member(pairs: Vec<Field>) -> Schema<Unknown> {
        Schema::as_unknown(
            Schema::<std::collections::BTreeMap<String, Unknown>>::struct_shape(pairs),
        )
    }

    /// `Schema` holds closures, so it has no `Debug`; a refusal is read out by
    /// hand rather than through `unwrap_err`.
    fn refusal(union: Schema<Unknown>, tag: &str) -> TaggedUnionError {
        match to_tagged_union(union, tag) {
            Ok(_) => panic!("expected the union to be refused"),
            Err(error) => error,
        }
    }

    #[test]
    fn a_number_and_its_spelling_are_one_key() {
        let union = Schema::union_with_mode(
            vec![member(vec![Field::tag("event", Unknown::Number(1.0))])],
            "anyOf",
        );
        let tagged = to_tagged_union(union, "event").expect("indexes");

        assert!(
            tagged.case_for(&Unknown::Text("1".to_string())).is_some(),
            "a number discriminant is reached by its spelling"
        );
        assert!(tagged.case_for(&Unknown::Number(1.0)).is_some());

        let clash = Schema::union_with_mode(
            vec![
                member(vec![Field::tag("event", Unknown::Number(1.0))]),
                member(vec![Field::tag("event", Unknown::Text("1".to_string()))]),
            ],
            "anyOf",
        );
        // The error carries the literal that arrived second, which is the one
        // the pin's message names.
        let error = refusal(clash, "event");
        assert_eq!(
            error,
            TaggedUnionError::Duplicate(Unknown::Text("1".to_string()))
        );
        assert_eq!(error.to_string(), "Duplicate discriminant: 1");
    }

    #[test]
    fn a_nested_union_is_descended_and_refuses_a_repeat() {
        let nested = Schema::union_with_mode(
            vec![
                member(vec![tag("event", "A")]),
                member(vec![tag("event", "B")]),
            ],
            "anyOf",
        );
        let union = Schema::union_with_mode(vec![member(vec![tag("event", "A")]), nested], "anyOf");

        assert_eq!(
            refusal(union, "event"),
            TaggedUnionError::Duplicate(Unknown::Text("A".to_string())),
            "the repeat lives in the nested union, not in one member list"
        );
    }
}
