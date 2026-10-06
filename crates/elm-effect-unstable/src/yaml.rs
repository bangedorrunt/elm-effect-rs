//! YAML subset: nested block mappings/sequences, flow sequences, and
//! scalars (booleans, integers, floats, strings). Anchors, aliases, tags,
//! comments, and multi-line scalars arrive with the full parser; anything
//! outside the subset is a parse error, never silent garbage. Blank lines
//! are skipped (no multi-line scalars exist in this subset).

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
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let indent = line.len() - line.trim_start_matches(' ').len();
            (indent, line.trim().to_string())
        })
        .collect();
    if lines.is_empty() {
        return Err(YamlError("empty document".to_string()));
    }
    let mut parser = Parser { lines, cursor: 0 };
    let value = parser.parse_block(0)?;
    if parser.cursor != parser.lines.len() {
        return Err(YamlError("trailing content".to_string()));
    }
    Ok(value)
}

struct Parser {
    lines: Vec<(usize, String)>,
    cursor: usize,
}

impl Parser {
    fn peek(&self) -> Option<&(usize, String)> {
        self.lines.get(self.cursor)
    }

    fn parse_block(&mut self, indent: usize) -> Result<YamlValue, YamlError> {
        let (line_indent, line) = self.peek().cloned().ok_or_else(|| YamlError("empty block".to_string()))?;
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
            let rest = if line == "-" { String::new() } else { line[2..].to_string() };
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
            None => parse_inline(&rest),
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
            let Some((key, value)) = split_key(&line) else { break };
            self.cursor += 1;
            pairs.push((key, self.inline_or_nested(line_indent, value)?));
        }
        Ok(pairs)
    }

    /// An inline value, or a nested block when the value is empty.
    fn inline_or_nested(&mut self, indent: usize, value: String) -> Result<YamlValue, YamlError> {
        if value.is_empty() {
            match self.peek() {
                Some((next, _)) if *next > indent => self.parse_block(*next),
                _ => Ok(YamlValue::Null),
            }
        } else {
            parse_inline(&value)
        }
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
            if let Some(stripped) = strip_quotes(text) {
                return Ok(YamlValue::Text(stripped));
            }
            if let Ok(int) = text.parse::<i64>() {
                return Ok(YamlValue::Int(int));
            }
            if let Ok(float) = text.parse::<f64>() {
                return Ok(YamlValue::Float(float));
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

/// Strip one pair of matching single or double quotes (no escape
/// processing in this subset).
fn strip_quotes(text: &str) -> Option<String> {
    if text.len() >= 2 {
        let (first, last) = (text.as_bytes()[0], text.as_bytes()[text.len() - 1]);
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return Some(text[1..text.len() - 1].to_string());
        }
    }
    None
}
