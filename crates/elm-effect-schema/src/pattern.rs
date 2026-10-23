//! The ECMAScript regular-expression subset behind `isPattern`: a parser, a
//! matcher, and the constructive generator `Arbitrary` drives.
//!
//! Strings are built from the pattern rather than filtered into it: the parser
//! turns the source into a small AST, a length analysis says which UTF-16
//! lengths the AST can produce, and the constructor picks one length and fills
//! it. Lengths are UTF-16 code units throughout, because that is what the
//! pin's `String.length` and its `isMaxLength` / `isBetweenLength` checks count.
//!
//! A construct outside the subset parses to `None` rather than a guess: a
//! lookahead, a backreference, a `\p{…}` class, or a case-folding flag would
//! otherwise generate strings that silently ignore part of the pattern. The
//! caller keeps such a pattern as a residual filter, which is the pin's own
//! arrangement.
//!
//! Two things differ from JavaScript, both because Rust has no UTF-16 strings:
//! matching walks code points, so a surrogate pair counts as one unit (the
//! pin's parser only reaches astral code points under the `u` flag, where
//! JavaScript also matches per code point), and a lone surrogate in the source
//! is refused because no Rust string can hold one.

use std::collections::{BTreeSet, HashMap};

/// The largest code point Unicode assigns.
const MAXIMUM_CODE_POINT: u32 = 0x10_ffff;
const HIGH_SURROGATE_MINIMUM: u32 = 0xd800;
const LOW_SURROGATE_MAXIMUM: u32 = 0xdfff;

/// How often a broad character class prefers a common code point. The pin
/// derives this from the attempt number; a fixed small factor keeps the same
/// "sometimes common, sometimes anywhere" shape without a run counter.
const BIAS_FACTOR: u32 = 2;

/// A closed range of code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    /// First code point.
    pub minimum: u32,
    /// Last code point.
    pub maximum: u32,
}

/// One node of the compiled pattern.
#[derive(Clone, Debug)]
enum Node {
    /// A run of literal code points (empty for an empty span).
    Literal(String),
    /// One code point from a class, with the bias the pin gives a broad class.
    Character(Character),
    /// In-order parts.
    Concatenation(Vec<Node>),
    /// One of several parts.
    Alternation(Vec<Node>),
    /// A repeated part: `minimum` to `maximum` copies (`None` is unbounded).
    Repetition {
        node: Box<Node>,
        minimum: usize,
        maximum: Option<usize>,
    },
    /// A zero-width assertion: the inner pattern must (or must not) match
    /// here. Matching enforces it; construction cannot aim at it, which is
    /// why a pattern holding one stays a residual filter.
    Lookahead { node: Box<Node>, negative: bool },
}

/// A code-point class plus the two facts generation needs: which preferred
/// code points it holds of each UTF-16 length, and how many code points of
/// each length it holds at all.
#[derive(Clone, Debug)]
struct Character {
    intervals: Vec<Interval>,
    preferred: [Vec<u32>; 2],
    counts: [usize; 2],
}

/// A parsed pattern.
#[derive(Clone, Debug)]
pub struct Pattern {
    /// The pattern without the unanchored surroundings, for matching.
    core: Node,
    /// The pattern with its surroundings, for constructing a full value.
    full: Node,
    source: String,
    flags: String,
    anchored_start: bool,
    anchored_end: bool,
    sticky: bool,
    minimum_length: usize,
    constructible: bool,
}

impl Pattern {
    /// Parse `source` with `flags`, or answer `None` for a construct outside
    /// the subset (and for a flag that would change what the pattern means).
    ///
    /// `g` is accepted and ignored: it decides whether a JavaScript `RegExp`
    /// object remembers a position, not what its pattern matches.
    pub fn parse(source: &str, flags: &str) -> Option<Self> {
        if flags
            .chars()
            .any(|flag| !matches!(flag, 'g' | 's' | 'u' | 'y'))
        {
            return None;
        }
        let unicode = flags.contains('u');
        let dot_all = flags.contains('s');
        let mut parser = Parser::new(source, dot_all);
        let parsed = parser.parse()?;
        let constructible = !parser.saw_lookahead;
        let core = if unicode {
            parsed
        } else {
            restrict_code_points(parsed, 0xffff)
        };
        let full = surrounding(&core, source, flags);
        let minimum_length = minimum_length(&full);
        Some(Self {
            core,
            full,
            source: source.to_string(),
            flags: flags.to_string(),
            anchored_start: source.starts_with('^'),
            anchored_end: has_trailing_anchor(source),
            sticky: flags.contains('y'),
            minimum_length,
            constructible,
        })
    }

    /// Whether the pattern can be constructed for. A pattern with a lookahead
    /// matches normally but generates nothing, so a caller keeps it as a
    /// residual filter.
    pub fn is_constructible(&self) -> bool {
        self.constructible
    }

    /// The pattern text, without slashes or flags.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The flags the pattern was parsed with.
    pub fn flags(&self) -> &str {
        &self.flags
    }

    /// The shortest value the pattern can produce, in UTF-16 code units. The
    /// unanchored surroundings can be empty, so this is the match's own floor.
    pub fn minimum_length(&self) -> usize {
        self.minimum_length
    }

    /// Whether the pattern matches somewhere in `value`, with the anchors and
    /// the sticky flag applied the way `RegExp.test` applies them.
    pub fn is_match(&self, value: &str) -> bool {
        let chars: Vec<char> = value.chars().collect();
        let starts: Vec<usize> = if self.sticky || self.anchored_start {
            vec![0]
        } else {
            (0..=chars.len()).collect()
        };
        let mut memo = HashMap::new();
        for start in starts {
            for end in match_ends(&self.core, &chars, start, &mut memo) {
                if !self.anchored_end || end == chars.len() {
                    return true;
                }
            }
        }
        false
    }

    /// Construct a value whose UTF-16 length is in `[minimum, maximum]`, or
    /// `None` when no length in that window is reachable.
    ///
    /// `random(n)` must answer a value in `[0, n)`.
    pub fn generate(
        &self,
        random: &mut dyn FnMut(u32) -> u32,
        minimum: usize,
        maximum: usize,
    ) -> Option<String> {
        if !self.constructible || minimum > maximum {
            return None;
        }
        let lengths = possible_lengths(&self.full, maximum);
        let candidates: Vec<usize> = (minimum..=maximum)
            .filter(|length| lengths.get(*length) == Some(&true))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let length = candidates[random_length(
            random,
            0,
            candidates.len().saturating_sub(1),
        )];
        let value = generate_exact(&self.full, length, random)?;
        self.is_match(&value).then_some(value)
    }

    /// Shrink candidates for a value this pattern produced: a shorter canonical
    /// value, then the canonical value at the same length (which collapses a
    /// repetition to its minimum and an alternation to its first feasible
    /// branch).
    pub fn shrink(&self, value: &str, minimum: usize) -> Vec<String> {
        let length = utf16_len(value);
        if length < minimum {
            return Vec::new();
        }
        let lengths = possible_lengths(&self.full, length);
        let smaller: Vec<usize> = (minimum..length)
            .filter(|candidate| lengths.get(*candidate) == Some(&true))
            .collect();
        let mut candidates = Vec::new();
        if let Some(first) = smaller.first() {
            candidates.push(*first);
            candidates.push(smaller[(smaller.len() - 1) / 2]);
            candidates.push(smaller[smaller.len() - 1]);
        }
        candidates.push(length);
        let mut out = Vec::new();
        for candidate_length in candidates {
            let Some(candidate) = canonical_exact(&self.full, candidate_length) else {
                continue;
            };
            if candidate == value || out.contains(&candidate) || !self.is_match(&candidate) {
                continue;
            }
            out.push(candidate);
        }
        out
    }
}

/// UTF-16 code-unit length, which is the length the pin's string checks count.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Whether the source ends with an unescaped `$`.
fn has_trailing_anchor(source: &str) -> bool {
    if !source.ends_with('$') {
        return false;
    }
    let mut backslashes = 0;
    for byte in source[..source.len() - 1].chars().rev() {
        if byte != '\\' {
            break;
        }
        backslashes += 1;
    }
    backslashes % 2 == 0
}

// ---------------------------------------------------------------------------
// Character classes
// ---------------------------------------------------------------------------

fn digit_intervals() -> Vec<Interval> {
    vec![Interval {
        minimum: 48,
        maximum: 57,
    }]
}

fn word_intervals() -> Vec<Interval> {
    vec![
        Interval {
            minimum: 48,
            maximum: 57,
        },
        Interval {
            minimum: 65,
            maximum: 90,
        },
        Interval {
            minimum: 95,
            maximum: 95,
        },
        Interval {
            minimum: 97,
            maximum: 122,
        },
    ]
}

fn whitespace_intervals() -> Vec<Interval> {
    vec![
        Interval {
            minimum: 9,
            maximum: 13,
        },
        Interval {
            minimum: 32,
            maximum: 32,
        },
        Interval {
            minimum: 160,
            maximum: 160,
        },
        Interval {
            minimum: 0x1680,
            maximum: 0x1680,
        },
        Interval {
            minimum: 0x2000,
            maximum: 0x200a,
        },
        Interval {
            minimum: 0x2028,
            maximum: 0x2029,
        },
        Interval {
            minimum: 0x202f,
            maximum: 0x202f,
        },
        Interval {
            minimum: 0x205f,
            maximum: 0x205f,
        },
        Interval {
            minimum: 0x3000,
            maximum: 0x3000,
        },
        Interval {
            minimum: 0xfeff,
            maximum: 0xfeff,
        },
    ]
}

fn line_terminator_intervals() -> Vec<Interval> {
    vec![
        Interval {
            minimum: 10,
            maximum: 10,
        },
        Interval {
            minimum: 13,
            maximum: 13,
        },
        Interval {
            minimum: 0x2028,
            maximum: 0x2029,
        },
    ]
}

fn code_point_intervals() -> Vec<Interval> {
    vec![Interval {
        minimum: 0,
        maximum: MAXIMUM_CODE_POINT,
    }]
}

fn printable_ascii_intervals() -> Vec<Interval> {
    vec![Interval {
        minimum: 32,
        maximum: 126,
    }]
}

/// The code points a broad class reaches for before anything else: printable
/// ASCII, the whitespace controls, null, and a few landmarks above ASCII.
fn preferred_code_points() -> Vec<u32> {
    let mut out: Vec<u32> = (32..=126).collect();
    out.extend([9, 10, 11, 12, 13, 0, 0x80, 0xe9, 0x3a9, 0x1f600]);
    out
}

fn normalize_intervals(intervals: Vec<Interval>) -> Vec<Interval> {
    if intervals.len() <= 1 {
        return intervals;
    }
    let mut sorted = intervals;
    sorted.sort_by_key(|interval| (interval.minimum, interval.maximum));
    let mut out: Vec<Interval> = Vec::with_capacity(sorted.len());
    for interval in sorted {
        match out.last_mut() {
            None => out.push(interval),
            Some(previous) if interval.minimum > previous.maximum + 1 => out.push(interval),
            Some(previous) if interval.maximum > previous.maximum => {
                previous.maximum = interval.maximum;
            }
            Some(_) => {}
        }
    }
    out
}

fn restrict_intervals(intervals: &[Interval], maximum: u32) -> Vec<Interval> {
    intervals
        .iter()
        .filter(|interval| interval.minimum <= maximum)
        .map(|interval| Interval {
            minimum: interval.minimum,
            maximum: interval.maximum.min(maximum),
        })
        .collect()
}

fn complement_intervals(intervals: &[Interval]) -> Vec<Interval> {
    let normalized = normalize_intervals(intervals.to_vec());
    let mut out = Vec::new();
    let mut minimum = 0u32;
    for interval in &normalized {
        if minimum < interval.minimum {
            out.push(Interval {
                minimum,
                maximum: interval.minimum - 1,
            });
        }
        minimum = minimum.max(interval.maximum.saturating_add(1));
    }
    if minimum <= MAXIMUM_CODE_POINT {
        out.push(Interval {
            minimum,
            maximum: MAXIMUM_CODE_POINT,
        });
    }
    out
}

fn code_point_in_intervals(code_point: u32, intervals: &[Interval]) -> bool {
    for interval in intervals {
        if code_point < interval.minimum {
            return false;
        }
        if code_point <= interval.maximum {
            return true;
        }
    }
    false
}

fn is_surrogate(code_point: u32) -> bool {
    (HIGH_SURROGATE_MINIMUM..=LOW_SURROGATE_MAXIMUM).contains(&code_point)
}

fn code_point_length(code_point: u32) -> usize {
    if code_point > 0xffff { 2 } else { 1 }
}

fn count_code_points_in_interval(interval: &Interval, length: usize) -> usize {
    let minimum = if length == 1 {
        interval.minimum
    } else {
        interval.minimum.max(0x1_0000)
    };
    let maximum = if length == 1 {
        interval.maximum.min(0xffff)
    } else {
        interval.maximum
    };
    if minimum > maximum {
        return 0;
    }
    let span = (maximum - minimum + 1) as usize;
    if length == 1 && minimum <= LOW_SURROGATE_MAXIMUM && maximum >= HIGH_SURROGATE_MINIMUM {
        // Surrogates are not characters on their own, so the spanned count
        // loses them. They cannot appear in a Rust string.
        return span
            - ((maximum.min(LOW_SURROGATE_MAXIMUM) - minimum.max(HIGH_SURROGATE_MINIMUM) + 1)
                as usize);
    }
    span
}

fn code_point_at_offset(interval: &Interval, length: usize, offset: usize) -> u32 {
    let mut code_point = if length == 1 {
        interval.minimum
    } else {
        interval.minimum.max(0x1_0000)
    } + offset as u32;
    if length == 1 && code_point >= HIGH_SURROGATE_MINIMUM {
        code_point += LOW_SURROGATE_MAXIMUM - HIGH_SURROGATE_MINIMUM + 1;
    }
    code_point
}

/// A class node from a list of intervals.
fn class_node(intervals: Vec<Interval>) -> Node {
    Node::Character(character(intervals))
}

fn character(intervals: Vec<Interval>) -> Character {
    let intervals = normalize_intervals(intervals);
    let preferred_points = preferred_code_points();
    let preferred_one: Vec<u32> = preferred_points
        .iter()
        .copied()
        .filter(|code_point| {
            code_point_length(*code_point) == 1 && code_point_in_intervals(*code_point, &intervals)
        })
        .collect();
    let preferred_two: Vec<u32> = preferred_points
        .iter()
        .copied()
        .filter(|code_point| {
            code_point_length(*code_point) == 2 && code_point_in_intervals(*code_point, &intervals)
        })
        .collect();
    let count_one = intervals
        .iter()
        .map(|interval| count_code_points_in_interval(interval, 1))
        .sum();
    let count_two = intervals
        .iter()
        .map(|interval| count_code_points_in_interval(interval, 2))
        .sum();
    Character {
        intervals,
        preferred: [preferred_one, preferred_two],
        counts: [count_one, count_two],
    }
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

fn literal(value: &str) -> Node {
    Node::Literal(value.to_string())
}

/// Flatten a concatenation, folding adjacent literals together. An empty node
/// in the list disappears, so an anchor (`^`, `$`) is a no-op here: the
/// surrounding analysis is what keeps it meaningful.
fn concatenation(nodes: Vec<Node>) -> Node {
    let mut flattened: Vec<Node> = Vec::new();
    let mut pending = String::new();
    for node in nodes {
        match node {
            Node::Literal(value) => pending.push_str(&value),
            Node::Concatenation(inner) => {
                if !pending.is_empty() {
                    flattened.push(Node::Literal(std::mem::take(&mut pending)));
                }
                flattened.extend(inner);
            }
            other => {
                if !pending.is_empty() {
                    flattened.push(Node::Literal(std::mem::take(&mut pending)));
                }
                flattened.push(other);
            }
        }
    }
    if !pending.is_empty() {
        flattened.push(Node::Literal(pending));
    }
    match flattened.len() {
        0 => Node::Literal(String::new()),
        1 => flattened.pop().expect("one node"),
        _ => Node::Concatenation(flattened),
    }
}

fn alternation(nodes: Vec<Node>) -> Node {
    let mut flattened = Vec::new();
    for node in nodes {
        match node {
            Node::Alternation(inner) => flattened.extend(inner),
            other => flattened.push(other),
        }
    }
    if flattened.len() == 1 {
        flattened.pop().expect("one node")
    } else {
        Node::Alternation(flattened)
    }
}

fn repetition(node: Node, minimum: usize, maximum: Option<usize>) -> Node {
    Node::Repetition {
        node: Box::new(node),
        minimum,
        maximum,
    }
}

/// Rewrite every class so it holds nothing above `maximum`. A pattern without
/// the `u` flag is matched per UTF-16 code unit, so it never reaches an astral
/// code point.
fn restrict_code_points(node: Node, maximum: u32) -> Node {
    match node {
        Node::Literal(value) => Node::Literal(value),
        Node::Character(class) => class_node(restrict_intervals(&class.intervals, maximum)),
        Node::Concatenation(nodes) => {
            Node::Concatenation(nodes.into_iter().map(|n| restrict_code_points(n, maximum)).collect())
        }
        Node::Alternation(nodes) => {
            Node::Alternation(nodes.into_iter().map(|n| restrict_code_points(n, maximum)).collect())
        }
        Node::Repetition {
            node,
            minimum,
            maximum: most,
        } => Node::Repetition {
            node: Box::new(restrict_code_points(*node, maximum)),
            minimum,
            maximum: most,
        },
        Node::Lookahead { node, negative } => Node::Lookahead {
            node: Box::new(restrict_code_points(*node, maximum)),
            negative,
        },
    }
}

/// Add the text an unanchored pattern is allowed to sit inside: printable ASCII
/// before (unless `^` or `y` pins the start) and after (unless `$` pins the
/// end).
fn surrounding(core: &Node, source: &str, flags: &str) -> Node {
    let loosely = || {
        repetition(
            class_node(printable_ascii_intervals()),
            0,
            None,
        )
    };
    let mut nodes = Vec::new();
    if !flags.contains('y') && !source.starts_with('^') {
        nodes.push(loosely());
    }
    nodes.push(core.clone());
    if !has_trailing_anchor(source) {
        nodes.push(loosely());
    }
    concatenation(nodes)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    source: Vec<char>,
    index: usize,
    supported: bool,
    dot_all: bool,
    /// Set when a lookahead is parsed: matching still works, construction
    /// does not.
    saw_lookahead: bool,
}

impl Parser {
    fn new(source: &str, dot_all: bool) -> Self {
        Self {
            source: source.chars().collect(),
            index: 0,
            supported: true,
            dot_all,
            saw_lookahead: false,
        }
    }

    fn parse(&mut self) -> Option<Node> {
        let node = self.parse_alternation();
        (self.supported && self.index == self.source.len()).then_some(node)
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.index).copied()
    }

    fn at(&self, offset: usize) -> Option<char> {
        self.source.get(self.index + offset).copied()
    }

    fn parse_alternation(&mut self) -> Node {
        let mut nodes = vec![self.parse_concatenation()];
        while self.peek() == Some('|') {
            self.index += 1;
            nodes.push(self.parse_concatenation());
        }
        alternation(nodes)
    }

    fn parse_concatenation(&mut self) -> Node {
        let mut nodes = Vec::new();
        while let Some(current) = self.peek() {
            if current == ')' || current == '|' {
                break;
            }
            let Some(atom) = self.parse_atom() else {
                self.supported = false;
                break;
            };
            nodes.push(self.parse_quantifier(atom));
        }
        concatenation(nodes)
    }

    fn parse_atom(&mut self) -> Option<Node> {
        let token = self.source[self.index];
        self.index += 1;
        match token {
            // An anchor is a zero-width assertion. Generation keeps it
            // meaningful through the surrounding analysis, which reads the
            // source's ends; the node itself contributes nothing.
            '^' | '$' => Some(Node::Literal(String::new())),
            '.' => {
                let intervals = if self.dot_all {
                    code_point_intervals()
                } else {
                    complement_intervals(&line_terminator_intervals())
                };
                Some(class_node(intervals))
            }
            '[' => self.parse_class(),
            '(' => self.parse_group(),
            '\\' => self.parse_escape(false),
            '*' | '+' | '?' | '{' | ')' => None,
            other => Some(literal(&other.to_string())),
        }
    }

    fn parse_group(&mut self) -> Option<Node> {
        if self.peek() == Some('?') {
            self.index += 1;
            match self.peek() {
                Some(':') => self.index += 1,
                Some('<') => {
                    // A named group is fine; a lookbehind is not.
                    let next = self.at(1);
                    if next == Some('=') || next == Some('!') {
                        return None;
                    }
                    self.index += 1;
                    while self.peek().is_some_and(|c| c != '>') {
                        self.index += 1;
                    }
                    if self.peek() != Some('>') {
                        return None;
                    }
                    self.index += 1;
                }
                Some('=') | Some('!') => {
                    let negative = self.peek() == Some('!');
                    self.index += 1;
                    let node = self.parse_alternation();
                    if self.peek() != Some(')') {
                        return None;
                    }
                    self.index += 1;
                    self.saw_lookahead = true;
                    return Some(Node::Lookahead {
                        node: Box::new(node),
                        negative,
                    });
                }
                _ => return None,
            }
        }
        let node = self.parse_alternation();
        if self.peek() != Some(')') {
            return None;
        }
        self.index += 1;
        Some(node)
    }

    fn parse_quantifier(&mut self, node: Node) -> Node {
        let start = self.index;
        let (minimum, maximum) = match self.peek() {
            Some('*') => {
                self.index += 1;
                (0, None)
            }
            Some('+') => {
                self.index += 1;
                (1, None)
            }
            Some('?') => {
                self.index += 1;
                (0, Some(1))
            }
            Some('{') => {
                self.index += 1;
                let Some(from) = self.parse_natural() else {
                    self.index = start;
                    return node;
                };
                match self.peek() {
                    Some('}') => {
                        self.index += 1;
                        (from, Some(from))
                    }
                    Some(',') => {
                        self.index += 1;
                        let to = self.parse_natural();
                        if self.peek() != Some('}') {
                            self.index = start;
                            return node;
                        }
                        self.index += 1;
                        (from, to)
                    }
                    _ => {
                        self.index = start;
                        return node;
                    }
                }
            }
            _ => return node,
        };
        // A lazy quantifier takes the fewest copies; for construction that is
        // the same node, since the count is chosen from the feasible range.
        if self.peek() == Some('?') {
            self.index += 1;
        }
        repetition(node, minimum, maximum)
    }

    fn parse_natural(&mut self) -> Option<usize> {
        let start = self.index;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.index += 1;
        }
        if start == self.index {
            return None;
        }
        let text: String = self.source[start..self.index].iter().collect();
        text.parse().ok()
    }

    fn parse_class(&mut self) -> Option<Node> {
        let negative = self.peek() == Some('^');
        if negative {
            self.index += 1;
        }
        let mut intervals: Vec<Interval> = Vec::new();
        let mut first = true;
        while self.index < self.source.len() && (self.peek() != Some(']') || first) {
            first = false;
            let left = self.parse_class_atom()?;
            if self.peek() == Some('-') && self.at(1) != Some(']') {
                self.index += 1;
                let right = self.parse_class_atom()?;
                if left.len() != 1
                    || right.len() != 1
                    || left[0].minimum != left[0].maximum
                    || right[0].minimum != right[0].maximum
                    || left[0].minimum > right[0].minimum
                {
                    return None;
                }
                intervals.push(Interval {
                    minimum: left[0].minimum,
                    maximum: right[0].minimum,
                });
            } else {
                intervals.extend(left);
            }
        }
        if self.peek() != Some(']') {
            return None;
        }
        self.index += 1;
        let normalized = normalize_intervals(intervals);
        Some(class_node(if negative {
            complement_intervals(&normalized)
        } else {
            normalized
        }))
    }

    fn parse_class_atom(&mut self) -> Option<Vec<Interval>> {
        let token = self.peek()?;
        self.index += 1;
        if token == '\\' {
            return match self.parse_escape(true)? {
                Node::Character(class) => Some(class.intervals),
                Node::Literal(value) => self.literal_intervals(&value),
                _ => None,
            };
        }
        Some(vec![Interval {
            minimum: token as u32,
            maximum: token as u32,
        }])
    }

    fn literal_intervals(&self, value: &str) -> Option<Vec<Interval>> {
        let mut chars = value.chars();
        let first = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        Some(vec![Interval {
            minimum: first as u32,
            maximum: first as u32,
        }])
    }

    fn parse_escape(&mut self, in_class: bool) -> Option<Node> {
        let token = self.peek()?;
        self.index += 1;
        match token {
            'd' => Some(class_node(digit_intervals())),
            'D' => Some(class_node(complement_intervals(&digit_intervals()))),
            'w' => Some(class_node(word_intervals())),
            'W' => Some(class_node(complement_intervals(&word_intervals()))),
            's' => Some(class_node(whitespace_intervals())),
            'S' => Some(class_node(complement_intervals(&whitespace_intervals()))),
            // `\b` inside a class is a backspace; outside it is a word
            // boundary, an assertion the constructor cannot place.
            'b' => in_class.then(|| literal("\u{8}")),
            'B' => None,
            'n' => Some(literal("\n")),
            'r' => Some(literal("\r")),
            't' => Some(literal("\t")),
            'v' => Some(literal("\u{b}")),
            'f' => Some(literal("\u{c}")),
            '0' => (!self.peek().is_some_and(|c| c.is_ascii_digit())).then(|| literal("\0")),
            'c' => {
                let control = self.peek()?;
                self.index += 1;
                if !control.is_ascii_alphabetic() {
                    return None;
                }
                let value = (control.to_ascii_uppercase() as u32) % 32;
                char::from_u32(value).map(|c| literal(&c.to_string()))
            }
            'x' => self.parse_code_point(2),
            'u' => {
                if self.peek() == Some('{') {
                    self.index += 1;
                    let start = self.index;
                    while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                        self.index += 1;
                    }
                    if start == self.index || self.peek() != Some('}') {
                        return None;
                    }
                    let text: String = self.source[start..self.index].iter().collect();
                    self.index += 1;
                    let value = u32::from_str_radix(&text, 16).ok()?;
                    if value > MAXIMUM_CODE_POINT || is_surrogate(value) {
                        return None;
                    }
                    return char::from_u32(value).map(|c| literal(&c.to_string()));
                }
                self.parse_code_point(4)
            }
            'p' | 'P' | 'k' => None,
            other if other.is_ascii_digit() => None,
            other => Some(literal(&other.to_string())),
        }
    }

    fn parse_code_point(&mut self, length: usize) -> Option<Node> {
        if self.index + length > self.source.len() {
            return None;
        }
        let text: String = self.source[self.index..self.index + length].iter().collect();
        if !text.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        self.index += length;
        let value = u32::from_str_radix(&text, 16).ok()?;
        // A lone surrogate has no `char`, and the port has no way to carry one.
        char::from_u32(value).map(|c| literal(&c.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Every position the node can end at when it starts at `position`, without
/// regard to what follows it.
fn match_ends(
    node: &Node,
    chars: &[char],
    position: usize,
    memo: &mut HashMap<(usize, usize), Vec<usize>>,
) -> Vec<usize> {
    let key = (node as *const Node as usize, position);
    if let Some(cached) = memo.get(&key) {
        return cached.clone();
    }
    let ends = match node {
        Node::Literal(value) => {
            let mut end = position;
            let mut matched = true;
            for expected in value.chars() {
                match chars.get(end) {
                    Some(actual) if *actual == expected => end += 1,
                    _ => {
                        matched = false;
                        break;
                    }
                }
            }
            if matched { vec![end] } else { Vec::new() }
        }
        Node::Character(class) => match chars.get(position) {
            Some(actual)
                if class
                    .intervals
                    .iter()
                    .any(|interval| *actual as u32 >= interval.minimum && *actual as u32 <= interval.maximum) =>
            {
                vec![position + 1]
            }
            _ => Vec::new(),
        },
        Node::Concatenation(nodes) => {
            let mut current = BTreeSet::from([position]);
            for child in nodes {
                let mut next = BTreeSet::new();
                for at in &current {
                    next.extend(match_ends(child, chars, *at, memo));
                }
                current = next;
                if current.is_empty() {
                    break;
                }
            }
            current.into_iter().collect()
        }
        Node::Alternation(nodes) => {
            let mut out = BTreeSet::new();
            for child in nodes {
                out.extend(match_ends(child, chars, position, memo));
            }
            out.into_iter().collect()
        }
        Node::Lookahead { node: child, negative } => {
            let matched = !match_ends(child, chars, position, memo).is_empty();
            if matched != *negative {
                vec![position]
            } else {
                Vec::new()
            }
        }
        Node::Repetition {
            node: child,
            minimum,
            maximum,
        } => {
            // Each extra copy either consumes a code point or leaves the
            // position where it is, so `minimum + remaining + 1` copies is
            // enough to reach every reachable position.
            let cap = minimum
                .saturating_add(chars.len().saturating_sub(position))
                .saturating_add(1);
            let limit = maximum.unwrap_or(cap).min(cap);
            let mut current = BTreeSet::from([position]);
            let mut out = BTreeSet::new();
            if *minimum == 0 {
                out.extend(current.iter().copied());
            }
            for count in 1..=limit {
                let mut next = BTreeSet::new();
                for at in &current {
                    next.extend(match_ends(child, chars, *at, memo));
                }
                if next.is_empty() {
                    break;
                }
                current = next;
                if count >= *minimum {
                    out.extend(current.iter().copied());
                }
            }
            out.into_iter().collect()
        }
    };
    memo.insert(key, ends.clone());
    ends
}

// ---------------------------------------------------------------------------
// Length analysis
// ---------------------------------------------------------------------------

fn empty_lengths(limit: usize) -> Vec<bool> {
    vec![false; limit + 1]
}

fn concatenate_lengths(left: &[bool], right: &[bool], limit: usize) -> Vec<bool> {
    let mut out = empty_lengths(limit);
    for (left_length, left_ok) in left.iter().enumerate().take(limit + 1) {
        if !left_ok {
            continue;
        }
        for (right_length, right_ok) in right.iter().enumerate().take(limit + 1 - left_length) {
            if *right_ok {
                out[left_length + right_length] = true;
            }
        }
    }
    out
}

/// Where each node in `nodes` can end when it starts here, as a length set per
/// suffix: `out[index]` is the set for `nodes[index..]`.
fn sequence_suffix_lengths(nodes: &[Node], length: usize) -> Vec<Vec<bool>> {
    let mut out = vec![empty_lengths(length); nodes.len() + 1];
    out[nodes.len()][0] = true;
    for index in (0..nodes.len()).rev() {
        out[index] = concatenate_lengths(
            &possible_lengths(&nodes[index], length),
            &out[index + 1],
            length,
        );
    }
    out
}

fn repetition_lengths(repetition: &Node, child: &[bool], limit: usize) -> Vec<bool> {
    let (minimum, maximum) = match repetition {
        Node::Repetition {
            minimum, maximum, ..
        } => (*minimum, *maximum),
        _ => unreachable!("called with a non-repetition"),
    };
    let nullable = child[0];
    let smallest_positive = (1..=limit).find(|length| child[*length]).unwrap_or(limit + 1);
    let relevant = if smallest_positive > limit {
        0
    } else {
        limit / smallest_positive
    };
    let minimum = if nullable { 0 } else { minimum };
    let maximum = maximum.unwrap_or(relevant).min(relevant);
    if minimum > maximum {
        return empty_lengths(limit);
    }
    let mut out = empty_lengths(limit);
    let mut current = empty_lengths(limit);
    current[0] = true;
    for count in 0..=maximum {
        if count >= minimum {
            for length in 0..=limit {
                if current[length] {
                    out[length] = true;
                }
            }
        }
        if count < maximum {
            current = concatenate_lengths(&current, child, limit);
        }
    }
    out
}

fn possible_lengths(node: &Node, limit: usize) -> Vec<bool> {
    match node {
        Node::Literal(value) => {
            let mut out = empty_lengths(limit);
            let length = utf16_len(value);
            if length <= limit {
                out[length] = true;
            }
            out
        }
        Node::Character(class) => {
            let mut out = empty_lengths(limit);
            if limit >= 1 && class.counts[0] > 0 {
                out[1] = true;
            }
            if limit >= 2 && class.counts[1] > 0 {
                out[2] = true;
            }
            out
        }
        Node::Alternation(nodes) => {
            let mut out = empty_lengths(limit);
            for child in nodes {
                for (length, reachable) in possible_lengths(child, limit).iter().enumerate() {
                    if *reachable {
                        out[length] = true;
                    }
                }
            }
            out
        }
        Node::Concatenation(nodes) => {
            let mut current = empty_lengths(limit);
            current[0] = true;
            for child in nodes {
                current = concatenate_lengths(&current, &possible_lengths(child, limit), limit);
            }
            current
        }
        Node::Repetition { .. } => {
            repetition_lengths(node, &possible_lengths(child_of(node), limit), limit)
        }
        Node::Lookahead { .. } => {
            let mut out = empty_lengths(limit);
            out[0] = true;
            out
        }
    }
}

fn child_of(node: &Node) -> &Node {
    match node {
        Node::Repetition { node, .. } => node,
        _ => unreachable!("called with a non-repetition"),
    }
}

/// The repetition counts that reach exactly `length` code units.
fn repetition_counts(node: &Node, length: usize) -> Vec<usize> {
    let (minimum, maximum) = match node {
        Node::Repetition {
            minimum, maximum, ..
        } => (*minimum, *maximum),
        _ => unreachable!("called with a non-repetition"),
    };
    let child = possible_lengths(child_of(node), length);
    let nullable = child[0];
    let smallest_positive = (1..=length).find(|step| child[*step]).unwrap_or(length + 1);
    let relevant = if smallest_positive > length {
        0
    } else {
        length / smallest_positive
    };
    let minimum = if nullable { 0 } else { minimum };
    let maximum = maximum.unwrap_or(relevant).min(relevant);
    if minimum > maximum {
        return Vec::new();
    }
    let mut counts = Vec::new();
    let mut current = empty_lengths(length);
    current[0] = true;
    for count in 0..=maximum {
        if count >= minimum && current[length] {
            counts.push(count);
        }
        if count < maximum {
            current = concatenate_lengths(&current, &child, length);
        }
    }
    counts
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

fn random_index(random: &mut dyn FnMut(u32) -> u32, length: usize) -> usize {
    if length <= 1 {
        0
    } else {
        (random(length as u32) as usize).min(length - 1)
    }
}

fn random_integer(random: &mut dyn FnMut(u32) -> u32, minimum: usize, maximum: usize) -> usize {
    if minimum >= maximum {
        return minimum;
    }
    minimum + (random((maximum - minimum + 1) as u32) as usize).min(maximum - minimum)
}

/// A length in `[minimum, maximum]`, leaning on the two ends the way the pin's
/// run-dependent edge bias does.
fn random_length(random: &mut dyn FnMut(u32) -> u32, minimum: usize, maximum: usize) -> usize {
    if minimum >= maximum {
        return minimum;
    }
    if random(BIAS_FACTOR) == 0 {
        return if random(2) == 0 { minimum } else { maximum };
    }
    random_integer(random, minimum, maximum)
}

fn generate_character(
    class: &Character,
    length: usize,
    random: &mut dyn FnMut(u32) -> u32,
) -> Option<char> {
    let preferred = &class.preferred[length - 1];
    if !preferred.is_empty() && random(BIAS_FACTOR) == 0 {
        let code_point = preferred[random_index(random, preferred.len())];
        return char::from_u32(code_point);
    }
    let count = class.counts[length - 1];
    if count == 0 {
        return None;
    }
    let mut offset = random_integer(random, 0, count - 1);
    for interval in &class.intervals {
        let size = count_code_points_in_interval(interval, length);
        if offset < size {
            return char::from_u32(code_point_at_offset(interval, length, offset));
        }
        offset -= size;
    }
    None
}

fn generate_exact(
    node: &Node,
    length: usize,
    random: &mut dyn FnMut(u32) -> u32,
) -> Option<String> {
    match node {
        Node::Literal(value) => (utf16_len(value) == length).then(|| value.clone()),
        Node::Character(class) => generate_character(class, length, random).map(|c| c.to_string()),
        Node::Alternation(nodes) => {
            let candidates: Vec<&Node> = nodes
                .iter()
                .filter(|child| possible_lengths(child, length).get(length) == Some(&true))
                .collect();
            if candidates.is_empty() {
                return None;
            }
            generate_exact(candidates[random_index(random, candidates.len())], length, random)
        }
        Node::Concatenation(nodes) => generate_sequence(nodes, length, random),
        Node::Lookahead { .. } => None,
        Node::Repetition { .. } => {
            let counts = repetition_counts(node, length);
            if counts.is_empty() {
                return None;
            }
            let count = counts[random_index(random, counts.len())];
            let copies = vec![child_of(node).clone(); count];
            generate_sequence(&copies, length, random)
        }
    }
}

fn generate_sequence(
    nodes: &[Node],
    length: usize,
    random: &mut dyn FnMut(u32) -> u32,
) -> Option<String> {
    let suffix = sequence_suffix_lengths(nodes, length);
    let mut remaining = length;
    let mut out = String::new();
    for (index, node) in nodes.iter().enumerate() {
        let node_lengths = possible_lengths(node, remaining);
        let tail = &suffix[index + 1];
        let candidates: Vec<usize> = (0..=remaining)
            .filter(|node_length| {
                node_lengths.get(*node_length) == Some(&true)
                    && tail.get(remaining - *node_length) == Some(&true)
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let node_length = candidates[random_index(random, candidates.len())];
        out.push_str(&generate_exact(node, node_length, random)?);
        remaining -= node_length;
    }
    (remaining == 0).then_some(out)
}

fn canonical_character(class: &Character, length: usize) -> Option<char> {
    if let Some(code_point) = class.preferred[length - 1].first() {
        return char::from_u32(*code_point);
    }
    for interval in &class.intervals {
        if count_code_points_in_interval(interval, length) > 0 {
            return char::from_u32(code_point_at_offset(interval, length, 0));
        }
    }
    None
}

/// The first value of exactly `length` the node can produce: first feasible
/// alternative, fewest feasible repetitions, first character of a class.
fn canonical_exact(node: &Node, length: usize) -> Option<String> {
    match node {
        Node::Literal(value) => (utf16_len(value) == length).then(|| value.clone()),
        Node::Character(class) => canonical_character(class, length).map(|c| c.to_string()),
        Node::Alternation(nodes) => nodes
            .iter()
            .find(|child| possible_lengths(child, length).get(length) == Some(&true))
            .and_then(|child| canonical_exact(child, length)),
        Node::Concatenation(nodes) => canonical_sequence(nodes, length),
        Node::Lookahead { .. } => None,
        Node::Repetition { .. } => {
            let count = *repetition_counts(node, length).first()?;
            let copies = vec![child_of(node).clone(); count];
            canonical_sequence(&copies, length)
        }
    }
}

fn canonical_sequence(nodes: &[Node], length: usize) -> Option<String> {
    let suffix = sequence_suffix_lengths(nodes, length);
    let mut remaining = length;
    let mut out = String::new();
    for (index, node) in nodes.iter().enumerate() {
        let node_lengths = possible_lengths(node, remaining);
        let tail = &suffix[index + 1];
        let mut node_length = 0;
        while node_length <= remaining
            && !(node_lengths.get(node_length) == Some(&true)
                && tail.get(remaining - node_length) == Some(&true))
        {
            node_length += 1;
        }
        if node_length > remaining {
            return None;
        }
        out.push_str(&canonical_exact(node, node_length)?);
        remaining -= node_length;
    }
    (remaining == 0).then_some(out)
}

fn minimum_length(node: &Node) -> usize {
    match node {
        Node::Literal(value) => utf16_len(value),
        Node::Character(class) => {
            if class.intervals.is_empty() {
                usize::MAX
            } else if class
                .intervals
                .first()
                .is_some_and(|interval| interval.minimum > 0xffff)
            {
                2
            } else {
                1
            }
        }
        Node::Alternation(nodes) => nodes.iter().map(minimum_length).min().unwrap_or(usize::MAX),
        Node::Concatenation(nodes) => nodes
            .iter()
            .fold(0usize, |sum, child| sum.saturating_add(minimum_length(child))),
        Node::Lookahead { .. } => 0,
        Node::Repetition { .. } => {
            let (minimum, _) = match node {
                Node::Repetition {
                    minimum, maximum, ..
                } => (*minimum, maximum),
                _ => unreachable!(),
            };
            minimum_length(child_of(node)).saturating_mul(minimum)
        }
    }
}

// ---------------------------------------------------------------------------
// The `isPattern` argument's spelling
// ---------------------------------------------------------------------------

/// Read the source and flags out of an `isPattern` argument, which prints as
/// `new RegExp("<source>", "<flags>")` with the source JSON-quoted.
pub fn parse_regexp_argument(argument: &str) -> Option<(String, String)> {
    let rest = argument.strip_prefix("new RegExp(")?.strip_suffix(')')?;
    let mut chars = rest.chars().peekable();
    let source = read_json_string(&mut chars)?;
    let flags = match chars.peek() {
        None => String::new(),
        Some(',') => {
            chars.next();
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            let flags = read_json_string(&mut chars)?;
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            flags
        }
        Some(_) => return None,
    };
    chars.next().is_none().then_some((source, flags))
}

fn read_json_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<String> {
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let mut value = 0u32;
                    for _ in 0..4 {
                        value = value * 16 + chars.next()?.to_digit(16)?;
                    }
                    out.push(char::from_u32(value)?);
                }
                _ => return None,
            },
            other => out.push(other),
        }
    }
}
