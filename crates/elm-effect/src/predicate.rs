//! `Predicate` at the pin: type guards over the dynamic value domain, plus
//! the structural combinators the pin's `Predicate` module exposes.
//!
//! The pin's guards answer over JavaScript's `unknown`, so this module
//! carries a [`Value`] domain holding exactly the shapes the carried rows
//! construct. A guard is a plain function `&Value -> bool`; the typed
//! [`Predicate`](crate::Predicate) combinators (`tuple`, `struct_`) build
//! the product rows from ordinary predicates.
//!
//! Reductions, stated rather than hidden:
//! - A symbol is its registry key: the carried rows use `Symbol.for` with
//!   distinct keys, so key identity carries both `hasProperty` cases.
//! - A date is epoch milliseconds and a regexp its source: the carried rows
//!   only observe instance-ness, never the contents.
//! - A function and a promise are unit variants: the carried rows observe
//!   callability and thenability, never behavior.

/// The dynamic value domain the guards answer over.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    String(String),
    BigInt(i128),
    /// A registered symbol, identified by its registry key.
    Symbol(String),
    Array(Vec<Value>),
    Object(Record),
    Set(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Uint8Array(Vec<u8>),
    /// Epoch milliseconds.
    Date(i64),
    /// The regexp source.
    RegExp(String),
    /// An error message.
    Error(String),
    Function,
    Promise,
}

/// A property key: a string or a registered symbol.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    String(String),
    Symbol(String),
}

/// An object: string keys to values.
pub type Record = std::collections::BTreeMap<String, Value>;

/// `Predicate.isTruthy`: JavaScript truthiness. Falsy are false, the empty
/// string, zero (number or bigint), null, undefined and NaN.
pub fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Undefined => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::BigInt(n) => *n != 0,
        _ => true,
    }
}

/// `Predicate.isSet`.
pub fn is_set(value: &Value) -> bool {
    matches!(value, Value::Set(_))
}

/// `Predicate.isMap`.
pub fn is_map(value: &Value) -> bool {
    matches!(value, Value::Map(_))
}

/// `Predicate.isString`: a primitive string, not a boxed one.
pub fn is_string(value: &Value) -> bool {
    matches!(value, Value::String(_))
}

/// `Predicate.isNumber`: NaN is a number.
pub fn is_number(value: &Value) -> bool {
    matches!(value, Value::Number(_))
}

/// `Predicate.isBoolean`.
pub fn is_boolean(value: &Value) -> bool {
    matches!(value, Value::Bool(_))
}

/// `Predicate.isBigInt`.
pub fn is_bigint(value: &Value) -> bool {
    matches!(value, Value::BigInt(_))
}

/// `Predicate.isSymbol`.
pub fn is_symbol(value: &Value) -> bool {
    matches!(value, Value::Symbol(_))
}

/// `Predicate.isPropertyKey`: strings, numbers and symbols.
pub fn is_property_key(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Number(_) | Value::Symbol(_)
    )
}

/// `Predicate.isFunction`.
pub fn is_function(value: &Value) -> bool {
    matches!(value, Value::Function)
}

/// `Predicate.isUndefined`.
pub fn is_undefined(value: &Value) -> bool {
    matches!(value, Value::Undefined)
}

/// The complement of [`is_undefined`].
pub fn is_not_undefined(value: &Value) -> bool {
    !is_undefined(value)
}

/// `Predicate.isNull`.
pub fn is_null(value: &Value) -> bool {
    matches!(value, Value::Null)
}

/// The complement of [`is_null`].
pub fn is_not_null(value: &Value) -> bool {
    !is_null(value)
}

/// `Predicate.isNullish`.
pub fn is_nullish(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Undefined)
}

/// The complement of [`is_nullish`].
pub fn is_not_nullish(value: &Value) -> bool {
    !is_nullish(value)
}

/// `Predicate.isNever`: nothing is never.
pub fn is_never(_value: &Value) -> bool {
    false
}

/// `Predicate.isUnknown`: everything is unknown.
pub fn is_unknown(_value: &Value) -> bool {
    true
}

/// `Predicate.isObjectOrArray`: objects and arrays, but not null and not
/// functions.
pub fn is_object_or_array(value: &Value) -> bool {
    matches!(value, Value::Object(_) | Value::Array(_))
}

/// `Predicate.isObject`: plain objects only — not arrays, not functions,
/// not null.
pub fn is_object(value: &Value) -> bool {
    matches!(value, Value::Object(_))
}

/// `Predicate.isReadonlyObject`: the same shape as [`is_object`].
pub fn is_readonly_object(value: &Value) -> bool {
    is_object(value)
}

/// `Predicate.isObjectKeyword`: objects, arrays and functions.
pub fn is_object_keyword(value: &Value) -> bool {
    matches!(
        value,
        Value::Object(_) | Value::Array(_) | Value::Function
    )
}

/// `Predicate.hasProperty`: the value is object-like and holds the key.
/// Symbols sit beside string keys in the record namespace.
pub fn has_property(value: &Value, key: &Key) -> bool {
    match (value, key) {
        (Value::Object(record), Key::String(name)) => record.contains_key(name),
        (Value::Object(record), Key::Symbol(name)) => record.contains_key(name),
        _ => false,
    }
}

/// `Predicate.hasProperty(key)`: the curried, data-last form as a
/// [`Predicate`](crate::Predicate).
pub fn has_property_with(key: Key) -> crate::Predicate<Value> {
    crate::Predicate::make(move |value: &Value| has_property(value, &key))
}

/// `Predicate.isTagged`: an object whose `_tag` equals the tag.
pub fn is_tagged(value: &Value, tag: &str) -> bool {
    match value {
        Value::Object(record) => {
            matches!(record.get("_tag"), Some(Value::String(seen)) if seen == tag)
        }
        _ => false,
    }
}

/// `Predicate.isTagged(tag)`: the curried form.
pub fn is_tagged_with(tag: &str) -> crate::Predicate<Value> {
    let tag = tag.to_string();
    crate::Predicate::make(move |value: &Value| is_tagged(value, &tag))
}

/// `Predicate.isError`.
pub fn is_error(value: &Value) -> bool {
    matches!(value, Value::Error(_))
}

/// `Predicate.isUint8Array`.
pub fn is_uint8array(value: &Value) -> bool {
    matches!(value, Value::Uint8Array(_))
}

/// `Predicate.isDate`.
pub fn is_date(value: &Value) -> bool {
    matches!(value, Value::Date(_))
}

/// `Predicate.isRegExp`.
pub fn is_regexp(value: &Value) -> bool {
    matches!(value, Value::RegExp(_))
}

/// `Predicate.isIterable`: arrays, strings, sets and maps.
pub fn is_iterable(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(_) | Value::String(_) | Value::Set(_) | Value::Map(_)
    )
}

/// `Predicate.isPromise`: an object with BOTH `then` and `catch`, or a
/// promise value.
pub fn is_promise(value: &Value) -> bool {
    match value {
        Value::Promise => true,
        Value::Object(record) => {
            record.contains_key("then") && record.contains_key("catch")
        }
        _ => false,
    }
}

/// `Predicate.isPromiseLike`: an object with `then`, or a promise value.
pub fn is_promise_like(value: &Value) -> bool {
    match value {
        Value::Promise => true,
        Value::Object(record) => record.contains_key("then"),
        _ => false,
    }
}

/// `Predicate.isTupleOf`: the value is an array of exactly `length`.
pub fn is_tuple_of(value: &Value, length: usize) -> bool {
    matches!(value, Value::Array(items) if items.len() == length)
}

/// `Predicate.isTupleOf(length)`: the curried form.
pub fn is_tuple_of_with(length: usize) -> crate::Predicate<Value> {
    crate::Predicate::make(move |value: &Value| is_tuple_of(value, length))
}

/// `Predicate.isTupleOfAtLeast`: the value is an array of at least `length`.
pub fn is_tuple_of_at_least(value: &Value, length: usize) -> bool {
    matches!(value, Value::Array(items) if items.len() >= length)
}

/// `Predicate.isTupleOfAtLeast(length)`: the curried form.
pub fn is_tuple_of_at_least_with(length: usize) -> crate::Predicate<Value> {
    crate::Predicate::make(move |value: &Value| is_tuple_of_at_least(value, length))
}
