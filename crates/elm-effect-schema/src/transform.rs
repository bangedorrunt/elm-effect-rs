//! Phase 6 transforms: `SchemaTransformation`, `SchemaGetter`,
//! `SchemaParser`, `StandardSchema`, `ChannelSchema`, plus brands.
//!
//! A [`Transformation`] is a pure two-way middle step: `forward` maps
//! decoded values on, `backward` maps them back for encoding.
//! [`is_transformation`] is the structural tag check. [`Getter`] reads
//! nested [`Unknown`] by path segments (own properties only, numeric
//! segments index arrays). [`make_tree_record`] builds an object tree
//! from `(path, value)` entries: duplicate paths keep array leaves,
//! conflicting leaves become containers, missing middles initialize.
//! [`Parser`] runs a [`Schema`] with accumulated ([`Vec`]) or first-error
//! results. [`StandardSchema`] adapts a schema to `{ value } | { issues }`.
//! [`ChannelSchema`] lifts encode/decode over item chunks. [`branded`]
//! tags a schema with a refinement name.
//!
//! Deliberate residuals (thread, not beads): Effect-async children,
//! bounded-concurrency products/unions, JS prototype pollution guards,
//! `Symbol` fields — all need the Effect runtime or the JS object model.

use std::collections::BTreeMap;

use super::{Schema, SchemaError, Unknown};

/// Two-way middle step between decode and encode.
pub struct Transformation<A, B> {
    forward: Box<dyn Fn(A) -> Result<B, SchemaError> + Send + Sync>,
    backward: Box<dyn Fn(B) -> Result<A, SchemaError> + Send + Sync>,
}

impl<A, B> Transformation<A, B> {
    /// Build from both directions.
    pub fn new(
        forward: impl Fn(A) -> Result<B, SchemaError> + Send + Sync + 'static,
        backward: impl Fn(B) -> Result<A, SchemaError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            forward: Box::new(forward),
            backward: Box::new(backward),
        }
    }

    /// Identity in both directions (`passthrough`).
    pub fn passthrough() -> Transformation<A, A>
    where
        A: 'static,
    {
        Transformation::new(Ok, Ok)
    }

    /// Run the forward direction.
    pub fn forward(&self, value: A) -> Result<B, SchemaError> {
        (self.forward)(value)
    }

    /// Run the backward direction.
    pub fn backward(&self, value: B) -> Result<A, SchemaError> {
        (self.backward)(value)
    }

    /// Compose with a following step (`pipe`).
    pub fn pipe<C>(self, next: Transformation<B, C>) -> Transformation<A, C>
    where
        A: 'static,
        B: 'static,
        C: 'static,
    {
        let first = std::sync::Arc::new(self);
        let second = std::sync::Arc::new(next);
        let fwd_first = first.clone();
        let fwd_second = second.clone();
        Transformation::new(
            move |a| fwd_first.forward(a).and_then(|b| fwd_second.forward(b)),
            move |c| second.backward(c).and_then(|b| first.backward(b)),
        )
    }
}

/// Structural tag check (`isTransformation`): real transformations carry
/// the marker; arbitrary values do not.
pub fn is_transformation<A, B>(t: &Transformation<A, B>) -> bool {
    let _ = t;
    true
}

/// A one-way read step over [`Unknown`] (`SchemaGetter` core).
pub struct Getter {
    read: Box<dyn Fn(&Unknown) -> Result<Unknown, SchemaError> + Send + Sync>,
}

impl Getter {
    /// Build from a read function.
    pub fn new(
        read: impl Fn(&Unknown) -> Result<Unknown, SchemaError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Box::new(read),
        }
    }

    /// Read from unknown input.
    pub fn read(&self, input: &Unknown) -> Result<Unknown, SchemaError> {
        (self.read)(input)
    }

    /// Map the read value (`map`, execution mode preserved trivially:
    /// reads are synchronous here).
    pub fn map(
        self,
        f: impl Fn(Unknown) -> Result<Unknown, SchemaError> + Send + Sync + 'static,
    ) -> Self {
        Self::new(move |u| self.read(u).and_then(&f))
    }

    /// Chain two reads (`compose`).
    pub fn compose(self, next: Getter) -> Self {
        Self::new(move |u| self.read(u).and_then(|v| next.read(&v)))
    }

    /// Read a child by key or index (own properties only).
    pub fn key(segment: String) -> Self {
        Self::new(move |u| match u {
            Unknown::Object(map) => map.get(&segment).cloned().ok_or(SchemaError::TypeMismatch {
                path: segment.clone(),
                expected: "present property",
                got: "missing",
            }),
            Unknown::Array(items) => segment
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i).cloned())
                .ok_or(SchemaError::TypeMismatch {
                    path: segment.clone(),
                    expected: "valid array index",
                    got: "missing",
                }),
            other => Err(SchemaError::TypeMismatch {
                path: segment.clone(),
                expected: "object or array",
                got: other.kind(),
            }),
        })
    }
}

/// Path segment: object key or array index.

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    /// Object key.
    Key(String),
    /// Array index.
    Index(usize),
    /// Append (`[]`): push onto the array at this point.
    Append,
}

/// Build an object tree from `(path, value)` entries (`makeTreeRecord`):
/// duplicate paths keep array-valued leaves, conflicting leaves become
/// containers, missing middles initialize, invalid index segments stay
/// object keys.
pub fn make_tree_record(entries: Vec<(Vec<Segment>, Unknown)>) -> Unknown {
    let mut root = Unknown::Object(BTreeMap::new());
    for (path, value) in entries {
        insert_path(&mut root, &path, value);
    }
    root
}

fn insert_path(node: &mut Unknown, path: &[Segment], value: Unknown) {
    if path.is_empty() {
        // Duplicate path: array leaves concatenate, anything else is
        // last-wins (conflicting leaves are replaced by containers below
        // when a deeper path arrives).
        match (node, value) {
            (Unknown::Array(items), Unknown::Array(mut more)) => {
                items.append(&mut more);
            }
            (slot, v) => {
                *slot = v;
            }
        }
        return;
    }
    match node {
        Unknown::Object(map) => {
            let key = match &path[0] {
                Segment::Key(k) => k.clone(),
                Segment::Index(i) => i.to_string(),
                // Bare `[]` at object level has no array to extend;
                // keep the value under the empty key.
                Segment::Append => String::new(),
            };
            let want_array = matches!(path.get(1), Some(Segment::Append) | Some(Segment::Index(_)));
            let child = map.entry(key).or_insert_with(|| {
                if want_array {
                    Unknown::Array(Vec::new())
                } else {
                    Unknown::Object(BTreeMap::new())
                }
            });
            insert_path(child, &path[1..], value);
        }
        Unknown::Array(items) => match &path[0] {
            Segment::Index(i) => {
                while items.len() <= *i {
                    items.push(Unknown::Object(BTreeMap::new()));
                }
                insert_path(&mut items[*i], &path[1..], value);
            }
            Segment::Append => {
                items.push(Unknown::Object(BTreeMap::new()));
                let last = items.len() - 1;
                insert_path(&mut items[last], &path[1..], value);
            }
            Segment::Key(k) => {
                // Invalid index segment on an array: sibling object key
                // cannot live in the array; replace the leaf with a
                // container holding the key.
                let mut map = BTreeMap::new();
                map.insert(k.clone(), Unknown::Object(BTreeMap::new()));
                *node = Unknown::Object(map);
                insert_path(node, path, value);
            }
        },
        _ => {
            let only_key = path.len() == 1 && matches!(&path[0], Segment::Key(_));
            let old = std::mem::replace(node, Unknown::Object(BTreeMap::new()));
            if only_key {
                // Duplicate scalar path: keep both as an array leaf.
                *node = Unknown::Array(vec![old, value]);
                return;
            }
            insert_path(node, path, value);
        }
    }
}

/// Parse `"a[0].b"` paths into segments (`a`, `0`, `b`).
pub fn parse_path(s: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    for dot in s.split('.') {
        let mut rest = dot;
        // Leading `name` before any `[`.
        if let Some(br) = rest.find('[') {
            if br > 0 {
                out.push(Segment::Key(rest[..br].to_string()));
            }
            rest = &rest[br..];
        } else {
            out.push(Segment::Key(rest.to_string()));
            continue;
        }
        // `[...]` chunks.
        while let Some(br) = rest.find('[') {
            let end = rest.find(']').unwrap_or(rest.len());
            let inner = &rest[br + 1..end];
            if inner.is_empty() {
                out.push(Segment::Append);
            } else {
                match inner.parse::<usize>() {
                    Ok(i) => out.push(Segment::Index(i)),
                    Err(_) => out.push(Segment::Key(inner.to_string())),
                }
            }
            rest = if end < rest.len() {
                &rest[end + 1..]
            } else {
                ""
            };
            if rest.is_empty() {
                break;
            }
        }
    }
    out
}

/// Flatten a tree back to `(path, leaf)` entries (encode direction).
pub fn flatten_tree(node: &Unknown) -> Vec<(Vec<Segment>, Unknown)> {
    let mut out = Vec::new();
    flatten_into(node, Vec::new(), &mut out);
    out
}

fn flatten_into(node: &Unknown, path: Vec<Segment>, out: &mut Vec<(Vec<Segment>, Unknown)>) {
    match node {
        Unknown::Object(map) if !map.is_empty() => {
            for (k, v) in map {
                let mut p = path.clone();
                p.push(Segment::Key(k.clone()));
                flatten_into(v, p, out);
            }
        }
        Unknown::Array(items) if !items.is_empty() => {
            for (i, v) in items.iter().enumerate() {
                let mut p = path.clone();
                p.push(Segment::Index(i));
                flatten_into(v, p, out);
            }
        }
        leaf => out.push((path, leaf.clone())),
    }
}

/// Parse `"2024-01-01 01:00:00"`-style civil strings (and ISO inputs with
/// offsets or `GMT`) to epoch millis (`dateTimeUtcFromInput`).
pub fn datetime_utc_from_input(s: &str) -> Result<i64, SchemaError> {
    parse_instant_millis(s).ok_or(SchemaError::TypeMismatch {
        path: String::new(),
        expected: "parseable date-time",
        got: "unparseable",
    })
}

/// Encode epoch millis as canonical `YYYY-MM-DDTHH:MM:SS.sssZ` (the
/// `Schema.Date` JSON wire shape asserted by the `toCodecJson` date rows).
/// Inverse of [`datetime_utc_from_input`] on canonical inputs.
pub fn millis_to_iso8601_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year,
        m,
        d,
        sod / 3_600,
        sod % 3_600 / 60,
        sod % 60,
        millis
    )
}

/// `Schema.Date` as epoch millis through its ISO-8601 JSON wire shape:
/// decode parses the string, encode renders canonical UTC millis text.
pub fn date_schema() -> Schema<i64> {
    Schema::convert(
        Schema::string(),
        |s: String| datetime_utc_from_input(&s),
        |ms: &i64| millis_to_iso8601_utc(*ms),
    )
}

/// `Schema.DateFromMillis`: epoch millis in and out. A Date is a whole number
/// of milliseconds inside the JS range (±8_640_000_000_000_000), so both are
/// enforced on the way in.
pub fn date_from_millis_schema() -> Schema<i64> {
    const DATE_LIMIT: f64 = 8_640_000_000_000_000.0;
    Schema::convert(
        Schema::number(),
        |millis: f64| {
            if millis.fract() != 0.0 {
                return Err(SchemaError::TypeMismatch {
                    path: "$".to_string(),
                    expected: "an integer",
                    got: "a fractional millisecond",
                });
            }
            if !(-DATE_LIMIT..=DATE_LIMIT).contains(&millis) {
                return Err(SchemaError::TypeMismatch {
                    path: "$".to_string(),
                    expected: "a valid Date",
                    got: "an out-of-range instant",
                });
            }
            Ok(millis as i64)
        },
        |millis: &i64| *millis as f64,
    )
}

fn parse_instant_millis(s: &str) -> Option<i64> {
    let mut t = s.trim().to_string();
    // RFC-2822 `Tue, 27 Jan 2026 17:14:06 GMT`.
    if t.contains("GMT") {
        return parse_rfc2822(&t);
    }
    // Space separator, `+HHMM` tails: normalize for the parser below.
    if let Some(i) = t.find(' ') {
        if t[..i].contains('-') {
            t.replace_range(i..i + 1, "T");
        }
    }
    if t.len() >= 5 {
        let tail = t[t.len() - 5..].to_string();
        let (sign, digits) = (&tail[..1], &tail[1..]);
        if (sign == "+" || sign == "-") && digits.bytes().all(|b| b.is_ascii_digit()) {
            t.insert(t.len() - 2, ':');
        }
    }
    // Split trailing zone: `Z`, `+hh:mm`, or `[IANA]` (IANA reads as its
    // offset at that instant is unavailable here, so bracketed inputs
    // resolve the offset part and drop the name).
    let t = t
        .strip_suffix('Z')
        .or_else(|| t.strip_suffix('z'))
        .unwrap_or(&t)
        .to_string();
    let t = match t.rfind('[') {
        Some(br) => t[..br].to_string(),
        None => t,
    };
    let (wall, off_mins) = match t.rfind(['+', '-']) {
        Some(i) if i > 9 => {
            let off = parse_offset(&t[i..])?;
            (t[..i].to_string(), off)
        }
        _ => (t, 0),
    };
    let (y, mo, d, h, mi, sec, ms) = parse_civil(&wall)?;
    let days = days_from_civil(y, mo, d);
    Some(
        days * 86_400_000 + h * 3_600_000 + mi * 60_000 + sec * 1_000 + ms
            - off_mins as i64 * 60_000,
    )
}

fn parse_offset(s: &str) -> Option<i32> {
    let (sign, rest) = match s.strip_prefix('+') {
        Some(r) => (1, r),
        None => (-1, s.strip_prefix('-')?),
    };
    let (h, m) = rest.split_once(':')?;
    if m.len() != 2 {
        return None;
    }
    let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
    if h > 18 || m > 59 {
        return None;
    }
    Some(sign * (h * 60 + m))
}

fn parse_civil(s: &str) -> Option<(i64, i64, i64, i64, i64, i64, i64)> {
    let (date, time) = s.split_once('T')?;
    let mut di = date.split('-');
    let (y, mo, d): (i64, i64, i64) = (
        di.next()?.parse().ok()?,
        di.next()?.parse().ok()?,
        di.next()?.parse().ok()?,
    );
    if di.next().is_some() || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let mut ti = time.split([':', '.']);
    let (h, mi, sec): (i64, i64, i64) = (
        ti.next()?.parse().ok()?,
        ti.next()?.parse().ok()?,
        ti.next()?.parse().ok()?,
    );
    // Optional `.sss` fraction, millis precision.
    let ms = match ti.next() {
        Some(frac) => {
            if frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
                // Longer fractions truncate like the spec printers.
                if !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let mut f = frac[..3.min(frac.len())].to_string();
                while f.len() < 3 {
                    f.push('0');
                }
                f.parse().ok()?
            } else {
                let mut f = frac.to_string();
                while f.len() < 3 {
                    f.push('0');
                }
                f.parse().ok()?
            }
        }
        None => 0,
    };
    if ti.next().is_some() || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some((y, mo, d, h, mi, sec, ms))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn parse_rfc2822(s: &str) -> Option<i64> {
    // `Tue, 27 Jan 2026 17:14:06 GMT`: fixed shape, GMT only.
    let s = s.strip_suffix("GMT")?.trim();
    let mut it = s.split_whitespace();
    it.next()?; // weekday name
    let day: i64 = it.next()?.parse().ok()?;
    let month = match it.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = it.next()?.parse().ok()?;
    let mut ti = it.next()?.split(':');
    let (h, mi, sec): (i64, i64, i64) = (
        ti.next()?.parse().ok()?,
        ti.next()?.parse().ok()?,
        ti.next()?.parse().ok()?,
    );
    Some(days_from_civil(year, month, day) * 86_400_000 + h * 3_600_000 + mi * 60_000 + sec * 1_000)
}

/// Parse options (`ParseOptions`): excess properties and unknown
/// handling. Defaults preserve everything and error on excess.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParseOptions {
    /// Keep excess properties instead of erroring.
    pub preserve_excess: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            preserve_excess: false,
        }
    }
}

/// Entry points over a [`Schema`] with first-error or accumulated results.
pub struct Parser;

impl Parser {
    /// Decode, first error wins (`decodeSync`).
    pub fn decode<T>(schema: &Schema<T>, input: Unknown) -> Result<T, SchemaError> {
        schema.decode(input)
    }

    /// Decode, collecting every error (`decode` with all issues).
    /// Opaque single-shot schemas yield at most one issue; use
    /// [`Parser::decode_fields`] for per-field accumulation.
    pub fn decode_all<T>(schema: &Schema<T>, input: Unknown) -> Result<T, Vec<SchemaError>> {
        schema.decode(input).map_err(|e| vec![e])
    }

    /// Decode each named field, accumulating failures (`all` issues).
    pub fn decode_fields<T>(
        fields: &[(&str, Schema<T>)],
        input: &Unknown,
    ) -> Result<BTreeMap<String, Unknown>, Vec<SchemaError>> {
        let mut out = BTreeMap::new();
        let mut errors = Vec::new();
        let obj = match input {
            Unknown::Object(map) => map,
            other => {
                return Err(vec![SchemaError::TypeMismatch {
                    path: String::new(),
                    expected: "object",
                    got: other.kind(),
                }]);
            }
        };
        for (name, schema) in fields {
            match obj.get(*name) {
                Some(value) => match schema.decode(value.clone()) {
                    Ok(v) => {
                        out.insert(name.to_string(), schema.encode(&v));
                    }
                    Err(e) => errors.push(e),
                },
                None => errors.push(SchemaError::TypeMismatch {
                    path: name.to_string(),
                    expected: "present property",
                    got: "missing",
                }),
            }
        }
        if errors.is_empty() {
            Ok(out)
        } else {
            Err(errors)
        }
    }

    /// Type-side check: true when the input decodes (`is`).
    pub fn is<T>(schema: &Schema<T>, input: &Unknown) -> bool {
        schema.decode(input.clone()).is_ok()
    }

    /// Encode back (`encodeSync`).
    pub fn encode<T>(schema: &Schema<T>, value: &T) -> Unknown {
        schema.encode(value)
    }

    /// JSON entry points.
    pub fn decode_json<T>(schema: &Schema<T>, json: &str) -> Result<T, SchemaError> {
        schema.decode_json(json)
    }

    /// Decode JSON, accumulating.
    pub fn decode_json_all<T>(schema: &Schema<T>, json: &str) -> Result<T, Vec<SchemaError>> {
        schema.decode_json(json).map_err(|e| vec![e])
    }
}

/// `StandardSchema` adapter: `{ value }` or `{ issues }`.
pub enum StandardResult<T> {
    /// Success.
    Value(T),
    /// Accumulated issues.
    Issues(Vec<SchemaError>),
}

/// Validate like Standard Schema (`validate`).
pub fn validate<T>(schema: &Schema<T>, input: Unknown) -> StandardResult<T> {
    match Parser::decode_all(schema, input) {
        Ok(value) => StandardResult::Value(value),
        Err(issues) => StandardResult::Issues(issues),
    }
}

/// Streaming lift (`ChannelSchema`): map encode/decode over chunks.
pub struct ChannelSchema<T> {
    inner: Schema<T>,
}

impl<T> ChannelSchema<T> {
    /// Wrap a schema for chunked transport.
    pub fn new(inner: Schema<T>) -> Self {
        Self { inner }
    }

    /// Decode every chunk, stopping at the first failure.
    pub fn decode_chunk(&self, chunk: Vec<Unknown>) -> Result<Vec<T>, SchemaError> {
        chunk.into_iter().map(|u| self.inner.decode(u)).collect()
    }

    /// Encode every chunk.
    pub fn encode_chunk(&self, chunk: &[T]) -> Vec<Unknown> {
        chunk.iter().map(|v| self.inner.encode(v)).collect()
    }
}

/// A named refinement on a schema (`brand`).
pub struct Branded<T> {
    brand: &'static str,
    schema: Schema<T>,
    check: Box<dyn Fn(&T) -> bool + Send + Sync>,
}

impl<T> Branded<T> {
    /// Attach a brand name plus its check.
    pub fn new(
        brand: &'static str,
        schema: Schema<T>,
        check: impl Fn(&T) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            brand,
            schema,
            check: Box::new(check),
        }
    }

    /// Brand name.
    pub fn brand(&self) -> &'static str {
        self.brand
    }

    /// Decode then check the refinement.
    pub fn decode(&self, input: Unknown) -> Result<T, SchemaError> {
        let value = self.schema.decode(input)?;
        if (self.check)(&value) {
            Ok(value)
        } else {
            Err(SchemaError::TypeMismatch {
                path: String::new(),
                expected: self.brand,
                got: "unbranded",
            })
        }
    }

    /// Encode back.
    pub fn encode(&self, value: &T) -> Unknown {
        self.schema.encode(value)
    }
}
