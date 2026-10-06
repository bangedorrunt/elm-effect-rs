//! YAML subset: nested block mappings/sequences, flow sequences, plain
//! scalars (booleans, integers, floats, strings), anchors and aliases.
//! Tags and comments arrive with the full parser; anything outside the
//! subset is a parse error, never silent garbage.

use std::collections::HashMap;

/// A parsed YAML value.
#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Seq(Vec<YamlValue>),
    Map(Vec<(String, YamlValue)>),
}

/// Why parsing failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError(pub String);

/// Parse a document in the supported subset.
pub fn parse(text: &str) -> Result<YamlValue, YamlError> {
    let lines: Vec<(usize, String)> = text
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                // Blank sentinel: only block scalars consume these.
                (usize::MAX, String::new())
            } else {
                let indent = line.len() - line.trim_start_matches(' ').len();
                (indent, line.trim().to_string())
            }
        })
        .collect();
    if lines.iter().all(|(_, line)| line.is_empty()) {
        return Err(YamlError("empty document".to_string()));
    }
    let mut parser = Parser {
        lines,
        cursor: 0,
        anchors: HashMap::new(),
    };
    let value = parser.parse_block(0)?;
    while matches!(parser.lines.get(parser.cursor), Some((usize::MAX, _))) {
        parser.cursor += 1;
    }
    if parser.cursor != parser.lines.len() {
        return Err(YamlError("trailing content".to_string()));
    }
    Ok(value)
}

/// Cut a ` #` comment tail, ignoring `#` inside quotes; reports whether
/// anything was cut.
fn strip_inline_comment(value: &str) -> (String, bool) {
    let mut in_single = false;
    let mut in_double = false;
    let mut cut = value.len();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\'' && !in_double {
            in_single = !in_single;
        } else if c == b'"' && !in_single {
            in_double = !in_double;
        } else if c == b'#'
            && !in_single
            && !in_double
            && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t')
        {
            cut = if i > 0 { i - 1 } else { 0 };
            break;
        }
        i += 1;
    }
    (value[..cut].trim_end().to_owned(), cut < value.len())
}

/// True when the opening quote of `text` has its closer (`''`
/// counts as an escaped literal inside single quotes).
fn quotes_closed(text: &str) -> bool {
    let mut chars = text.chars();
    let quote = match chars.next() {
        Some(q) if q == '\'' || q == '"' => q,
        _ => return true,
    };
    let mut open = true;
    let mut chars = chars.peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && quote == '"' {
            chars.next();
        } else if c == '\'' && quote == '\'' && chars.peek() == Some(&'\'') {
            chars.next();
        } else if c == quote {
            open = !open;
        }
    }
    !open
}

/// True for lines that start nested structure rather than continuing
/// a plain scalar.
fn is_structural_line(line: &str) -> bool {
    line.starts_with("- ")
        || line == "-"
        || line.starts_with('|')
        || line.starts_with('>')
        || line.starts_with('&')
        || line.starts_with('*')
        || line.starts_with('[')
        || line.starts_with('{')
        || line.contains(": ")
        || line.ends_with(':')
}

/// Split a leading `&name` anchor from its remainder.
fn split_anchor(value: &str) -> Option<(String, String)> {
    let rest = value.strip_prefix('&')?;
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    Some((rest[..end].to_owned(), rest[end..].trim().to_string()))
}

/// Fold `>` lines: single breaks become spaces, breaks aroundndent
/// is preserved on the deeper line).
fn fold_block_lines(content: &[Option<(usize, String)>], base: usize) -> String {
    let mut out = String::new();
    let mut first = true;
    let mut pending_blanks = 0usize;
    let mut prev_deep = false;
    for item in content {
        match item {
            None => pending_blanks += 1,
            Some((line_indent, line)) => {
                let deep = *line_indent > base;
                if first {
                    if pending_blanks > 0 {
                        out.push_str(&"\n".repeat(pending_blanks));
                    }
                } else if pending_blanks > 0 {
                    out.push_str(&"\n".repeat(pending_blanks));
                } else if prev_deep || deep {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
                if deep {
                    out.push_str(&" ".repeat(line_indent - base));
                }
                out.push_str(line);
                pending_blanks = 0;
                prev_deep = deep;
                first = false;
            }
        }
    }
    out
}

struct Parser {
    lines: Vec<(usize, String)>,
    cursor: usize,
    anchors: HashMap<String, YamlValue>,
}

impl Parser {
    fn peek(&mut self) -> Option<&(usize, String)> {
        loop {
            match self.lines.get(self.cursor) {
                // Blank sentinels and full-line comments never start values.
                Some((usize::MAX, _)) => self.cursor += 1,
                Some((_, line)) if line.starts_with('#') => self.cursor += 1,
                _ => break,
            }
        }
        self.lines.get(self.cursor)
    }

    /// Raw lookahead including blank sentinels (block scalars only).
    fn peek_raw(&self) -> Option<&(usize, String)> {
        self.lines.get(self.cursor)
    }

    fn parse_block(&mut self, indent: usize) -> Result<YamlValue, YamlError> {
        let (line_indent, line) = self
            .peek()
            .cloned()
            .ok_or_else(|| YamlError("empty block".to_string()))?;
        if line_indent < indent {
            return Err(YamlError("bad indentation".to_string()));
        }
        if line == "-" || line.starts_with("- ") {
            Ok(YamlValue::Seq(self.parse_seq(line_indent)?))
        } else {
            Ok(YamlValue::Map(self.parse_map(line_indent)?))
        }
    }

    fn parse_seq(&mut self, indent: usize) -> Result<Vec<YamlValue>, YamlError> {
        let mut items = Vec::new();
        while let Some((line_indent, line)) = self.peek().cloned() {
            if line_indent != indent || !(line == "-" || line.starts_with("- ")) {
                break;
            }
            self.cursor += 1;
            let rest = if line == "-" {
                String::new()
            } else {
                line[2..].to_string()
            };
            items.push(self.parse_seq_item(indent, rest)?);
        }
        Ok(items)
    }

    /// A sequence item: either an inline value or a mapping whose first
    /// pair sits on the dash line with the rest nested deeper.
    fn parse_seq_item(&mut self, indent: usize, rest: String) -> Result<YamlValue, YamlError> {
        if rest.is_empty() {
            return self.parse_block(indent + 1).or(Ok(YamlValue::Null));
        }
        match split_key(&rest) {
            Some((key, value)) => {
                let mut pairs = vec![(key, self.inline_or_nested(indent, value)?)];
                pairs.extend(self.parse_map_rest(indent + 1)?);
                Ok(YamlValue::Map(pairs))
            }
            None => self.inline_or_nested(indent, rest),
        }
    }

    fn parse_map(&mut self, indent: usize) -> Result<Vec<(String, YamlValue)>, YamlError> {
        let mut pairs = Vec::new();
        while let Some((line_indent, line)) = self.peek().cloned() {
            if line_indent != indent {
                break;
            }
            let Some((key, value)) = split_key(&line) else {
                return Err(YamlError(format!("expected key: {line}")));
            };
            self.cursor += 1;
            if pairs.iter().any(|(existing, _)| *existing == key) {
                return Err(YamlError(format!("duplicate key: {key}")));
            }
            pairs.push((key, self.inline_or_nested(indent, value)?));
        }
        Ok(pairs)
    }

    /// Continuation pairs of an inline-started mapping, nested deeper.
    fn parse_map_rest(&mut self, min_indent: usize) -> Result<Vec<(String, YamlValue)>, YamlError> {
        let mut pairs = Vec::new();
        while let Some((line_indent, line)) = self.peek().cloned() {
            if line_indent < min_indent {
                break;
            }
            let Some((key, value)) = split_key(&line) else {
                break;
            };
            self.cursor += 1;
            pairs.push((key, self.inline_or_nested(line_indent, value)?));
        }
        Ok(pairs)
    }

    /// An inline value, or a nested block when the value is empty.
    fn inline_or_nested(&mut self, indent: usize, value: String) -> Result<YamlValue, YamlError> {
        let (value, head_comment) = strip_inline_comment(&value);
        if let Some(alias) = value.strip_prefix('*') {
            let name = alias.trim();
            return self
                .anchors
                .get(name)
                .cloned()
                .ok_or_else(|| YamlError(format!("unknown alias: {name}")));
        }
        if let Some((name, rest)) = split_anchor(&value) {
            let parsed = self.inline_or_nested(indent, rest)?;
            self.anchors.insert(name, parsed.clone());
            return Ok(parsed);
        }
        if value.is_empty() {
            match self.peek().cloned() {
                Some((next, _)) if next > indent => self.parse_block(next),
                // YAML 1.2.2 §8.2.1: a mapping value's block sequence may
                // sit at the key's own indent.
                Some((next, line)) if next == indent && (line.starts_with("- ") || line == "-") => {
                    self.parse_seq(next).map(YamlValue::Seq)
                }
                _ => Ok(YamlValue::Null),
            }
        } else if is_block_header(&value) {
            self.parse_block_scalar(indent, value)
        } else {
            let mut text = value;
            // Quoted scalars fold deeper lines until the quote closes.
            if (text.starts_with('"') || text.starts_with('\'')) && !quotes_closed(&text) {
                while !quotes_closed(&text) {
                    match self.peek().cloned() {
                        Some((next_indent, next_line)) if next_indent > indent => {
                            self.cursor += 1;
                            text.push(' ');
                            text.push_str(&next_line);
                        }
                        _ => break,
                    }
                }
            // Plain scalars fold deeper-indented non-structural lines.
            // A comment on the first line terminates the scalar: further
            // lines belong to following structure (or fail).
            } else if !head_comment
                && !text.starts_with(|c| c == '"' || c == '\'' || c == '[' || c == '{')
            {
                while let Some((next_indent, next_line)) = self.peek().cloned() {
                    if next_indent <= indent || is_structural_line(&next_line) {
                        break;
                    }
                    self.cursor += 1;
                    let (piece, piece_comment) = strip_inline_comment(&next_line);
                    text.push(' ');
                    text.push_str(&piece);
                    if piece_comment {
                        break;
                    }
                }
            }
            parse_inline(&text)
        }
    }

    /// A `|` literal or `>` folded block: content lines nested deeper
    /// than the key; `-` strips the trailing break, otherwise clipped to
    /// one; `+` keeps trailing breaks, digits pin the base indent.
    fn parse_block_scalar(
        &mut self,
        indent: usize,
        header: String,
    ) -> Result<YamlValue, YamlError> {
        let style = header.as_bytes()[0];
        let flags: String = header[1..].trim().to_string();
        let stripped = flags.contains('-');
        let keep = flags.contains('+');
        let digits: String = flags.chars().filter(|c| *c != '-' && *c != '+').collect();
        let explicit_base: Option<usize> = if digits.is_empty() {
            None
        } else {
            Some(
                digits
                    .parse()
                    .map_err(|_| YamlError(format!("bad block header: {header}")))?,
            )
        };
        // (indent, text) items; `None` marks a blank line inside the block.
        let mut content: Vec<Option<(usize, String)>> = Vec::new();
        let mut content_indent: Option<usize> = None;
        while let Some((line_indent, line)) = self.peek_raw().cloned() {
            if line_indent == usize::MAX {
                self.cursor += 1;
                content.push(None);
                continue;
            }
            if line.starts_with('#') {
                // Full-line comments vanish inside scalars too.
                self.cursor += 1;
                continue;
            }
            if line_indent <= indent {
                break;
            }
            // Explicit digits pin the floor; otherwise the first line does.
            let floor = explicit_base.unwrap_or(content_indent.unwrap_or(line_indent));
            if line_indent < floor {
                return Err(YamlError("bad block indentation".to_string()));
            }
            if content_indent.is_none() {
                content_indent = Some(line_indent);
            }
            self.cursor += 1;
            content.push(Some((line_indent, line)));
        }
        // Trailing blanks are chomping territory, not content.
        let mut trailing_blanks = 0usize;
        while matches!(content.last(), Some(None)) {
            content.pop();
            trailing_blanks += 1;
        }
        if content.is_empty() {
            return Ok(YamlValue::Text(String::new()));
        }
        let base = explicit_base.or(content_indent).unwrap_or(indent + 1);
        let mut body = if style == b'|' {
            content
                .iter()
                .map(|item| match item {
                    None => String::new(),
                    Some((_, line)) => line.clone(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            fold_block_lines(&content, base)
        };
        if keep {
            body.push_str(&"\n".repeat(trailing_blanks + 1));
        } else if !stripped {
            body.push('\n');
        }
        Ok(YamlValue::Text(body))
    }
}

/// Split `key: value` at the first colon followed by a space or
/// end-of-line (so `http://x` never splits). Returns the trimmed key.
fn split_key(line: &str) -> Option<(String, String)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ') {
            let key = line[..i].trim().to_string();
            let value = if i + 1 == bytes.len() {
                String::new()
            } else {
                line[i + 1..].trim().to_string()
            };
            if key.is_empty() {
                return None;
            }
            return Some((key, value));
        }
        i += 1;
    }
    None
}

/// One inline value: flow sequence, boolean, number, or string.
fn parse_inline(text: &str) -> Result<YamlValue, YamlError> {
    let text = text.trim();
    if let Some(inner) = text.strip_prefix('[') {
        let Some(body) = inner.strip_suffix(']') else {
            return Err(YamlError("unclosed flow sequence".to_string()));
        };
        return parse_flow(body);
    }
    match text {
        "true" => Ok(YamlValue::Bool(true)),
        "false" => Ok(YamlValue::Bool(false)),
        "null" | "~" => Ok(YamlValue::Null),
        _ => {
            if text.starts_with('\'') || text.starts_with('"') {
                return match strip_quotes(text) {
                    Some(stripped) => Ok(YamlValue::Text(stripped)),
                    None => Err(YamlError("bad quoted scalar".to_string())),
                };
            }
            if let Ok(int) = text.parse::<i64>() {
                return Ok(YamlValue::Int(int));
            }
            if let Ok(float) = text.parse::<f64>() {
                return Ok(YamlValue::Float(float));
            }
            if text.contains(": ") || text.contains(":\t") || text.ends_with(':') {
                return Err(YamlError("bad plain scalar".to_string()));
            }
            Ok(YamlValue::Text(text.to_string()))
        }
    }
}

/// Comma-separated flow items at one bracket level; nested `[...]`
/// recurses. Empty items are skipped.
fn parse_flow(body: &str) -> Result<YamlValue, YamlError> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0usize;
    while i <= chars.len() {
        let boundary = i == chars.len()
            || (chars[i] == ',' && depth == 0);
        if i < chars.len() {
            match chars[i] {
                '[' => depth += 1,
                ']' => {
                    if depth == 0 {
                        return Err(YamlError("unbalanced flow sequence".to_string()));
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        if boundary {
            let item: String = chars[start..i].iter().collect();
            let item = item.trim();
            if !item.is_empty() {
                items.push(parse_inline(item)?);
            }
            start = i + 1;
        }
        i += 1;
    }
    if depth != 0 {
        return Err(YamlError("unbalanced flow sequence".to_string()));
    }
    Ok(YamlValue::Seq(items))
}

/// A block-scalar header: `|`, `>`, optionally followed by chomping
/// flags, digits, or trailing space.
fn is_block_header(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || (bytes[0] != b'|' && bytes[0] != b'>') {
        return false;
    }
    let rest = &value[1..];
    rest.is_empty() || rest.starts_with(' ') || rest.starts_with('-') || rest.starts_with('+')
}

/// Strip one pair of matching quotes, processing escapes: double quotes
/// take `\\`, `\"`, `\n`, `\t`, `\r`, `\b`, `\f`, and `\uXXXX`;
/// single quotes take doubled `''` as one quote.
fn strip_quotes(text: &str) -> Option<String> {
    if text.len() >= 2 {
        let (first, last) = (text.as_bytes()[0], text.as_bytes()[text.len() - 1]);
        if first == b'\'' && last == b'\'' {
            return Some(text[1..text.len() - 1].replace("''", "'"));
        }
        if first == b'"' && last == b'"' {
            return unescape_double(&text[1..text.len() - 1]);
        }
    }
    None
}

/// Process double-quote escapes; `None` on malformed input.
fn unescape_double(body: &str) -> Option<String> {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(char) = chars.next() {
        if char != '\\' {
            out.push(char);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                if hex.len() != 4 {
                    return None;
                }
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    Some(out)
}
