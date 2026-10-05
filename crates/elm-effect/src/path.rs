//! `Path.ts` port: path service with fakes.
//!
//! [`Path`] is the service interface (separator plus pure path algebra);
//! [`UnixPath`] implements POSIX semantics, [`FakePath`] records calls
//! for tests. Parsed shapes mirror `path.parse` (`root`, `dir`, `base`,
//! `name`, `ext`). `file:` URLs convert without touching the network.

/// Parsed path shape (`path.parse`).
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    /// Filesystem root (`/` or empty).
    pub root: String,
    /// Containing directory.
    pub dir: String,
    /// Final component with extension.
    pub base: String,
    /// Final component without extension.
    pub name: String,
    /// Extension including the dot (empty when none).
    pub ext: String,
}

/// Path service interface.
pub trait Path: Send + Sync {
    /// Separator (`/` on POSIX).
    fn sep(&self) -> char;
    /// Final component, minus `suffix`.
    fn basename(&self, path: &str, suffix: Option<&str>) -> String;
    /// Containing directory.
    fn dirname(&self, path: &str) -> String;
    /// Extension with dot, else empty.
    fn extname(&self, path: &str) -> String;
    /// Join segments.
    fn join(&self, parts: &[&str]) -> String;
    /// Normalize `.`/`..`/doubled separators.
    fn normalize(&self, path: &str) -> String;
    /// True for absolute paths.
    fn is_absolute(&self, path: &str) -> bool;
    /// Parse into parts.
    fn parse(&self, path: &str) -> Parsed;
    /// Format parts back.
    fn format(&self, parsed: &Parsed) -> String;
    /// Relative path from `from` to `to`.
    fn relative(&self, from: &str, to: &str) -> String;
    /// Convert a `file:` URL string to a path.
    fn from_file_url(&self, url: &str) -> Result<String, String>;
}

/// POSIX path implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnixPath;

impl UnixPath {
    fn split(path: &str) -> (String, String) {
        match path.rfind('/') {
            Some(0) => ("/".to_string(), path[1..].to_string()),
            Some(i) => (path[..i].to_string(), path[i + 1..].to_string()),
            None => (String::new(), path.to_string()),
        }
    }

    fn strip_ext(base: &str) -> (String, String) {
        match base.rfind('.') {
            Some(0) | None => (base.to_string(), String::new()),
            Some(i) => (base[..i].to_string(), base[i..].to_string()),
        }
    }
}

impl Path for UnixPath {
    fn sep(&self) -> char {
        '/'
    }

    fn basename(&self, path: &str, suffix: Option<&str>) -> String {
        let (_, base) = Self::split(path.trim_end_matches('/'));
        match suffix {
            Some(s) if !s.is_empty() => base.strip_suffix(s).unwrap_or(&base).to_string(),
            _ => base,
        }
    }

    fn dirname(&self, path: &str) -> String {
        let (dir, _) = Self::split(path.trim_end_matches('/'));
        if dir.is_empty() {
            ".".to_string()
        } else {
            dir
        }
    }

    fn extname(&self, path: &str) -> String {
        let (_, base) = Self::split(path);
        Self::strip_ext(&base).1
    }

    fn join(&self, parts: &[&str]) -> String {
        let mut out = String::new();
        for part in parts {
            if part.is_empty() {
                continue;
            }
            if part.starts_with('/') {
                out = part.to_string();
            } else if out.is_empty() || out.ends_with('/') {
                out.push_str(part);
            } else {
                out.push('/');
                out.push_str(part);
            }
        }
        if out.is_empty() {
            ".".to_string()
        } else {
            out
        }
    }

    fn normalize(&self, path: &str) -> String {
        let absolute = path.starts_with('/');
        let mut stack: Vec<&str> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if stack.pop().is_none() && !absolute {
                        stack.push("..");
                    }
                }
                p => stack.push(p),
            }
        }
        let mut out = stack.join("/");
        if absolute {
            out.insert(0, '/');
        }
        if out.is_empty() {
            ".".to_string()
        } else {
            out
        }
    }

    fn is_absolute(&self, path: &str) -> bool {
        path.starts_with('/')
    }

    fn parse(&self, path: &str) -> Parsed {
        let root = if path.starts_with('/') { "/" } else { "" }.to_string();
        let (dir, base) = Self::split(path);
        let (name, ext) = Self::strip_ext(&base);
        Parsed { root, dir, base, name, ext }
    }

    fn format(&self, parsed: &Parsed) -> String {
        if parsed.dir.is_empty() || parsed.dir == "." {
            parsed.base.clone()
        } else if parsed.dir.ends_with('/') {
            format!("{}{}", parsed.dir, parsed.base)
        } else {
            format!("{}/{}", parsed.dir, parsed.base)
        }
    }

    fn relative(&self, from: &str, to: &str) -> String {
        let from: Vec<&str> = from.split('/').filter(|p| !p.is_empty()).collect();
        let to: Vec<&str> = to.split('/').filter(|p| !p.is_empty()).collect();
        let common = from.iter().zip(to.iter()).take_while(|(a, b)| a == b).count();
        let mut out: Vec<&str> = vec![".."; from.len() - common];
        out.extend(to[common..].iter().copied());
        if out.is_empty() {
            String::new()
        } else {
            out.join("/")
        }
    }

    fn from_file_url(&self, url: &str) -> Result<String, String> {
        url.strip_prefix("file://")
            .map(str::to_string)
            .ok_or_else(|| format!("not a file URL: {url}"))
    }
}

/// Recording fake: scripted outputs plus a call log.
#[derive(Clone, Debug, Default)]
pub struct FakePath {
    /// Recorded method names in order.
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    inner: UnixPath,
}

impl FakePath {
    /// Fresh fake delegating algebra to POSIX rules.
    pub fn new() -> Self {
        Self::default()
    }

    fn note(&self, name: &str) {
        self.calls.lock().unwrap().push(name.to_string());
    }
}

impl Path for FakePath {
    fn sep(&self) -> char {
        self.note("sep");
        self.inner.sep()
    }

    fn basename(&self, path: &str, suffix: Option<&str>) -> String {
        self.note("basename");
        self.inner.basename(path, suffix)
    }

    fn dirname(&self, path: &str) -> String {
        self.note("dirname");
        self.inner.dirname(path)
    }

    fn extname(&self, path: &str) -> String {
        self.note("extname");
        self.inner.extname(path)
    }

    fn join(&self, parts: &[&str]) -> String {
        self.note("join");
        self.inner.join(parts)
    }

    fn normalize(&self, path: &str) -> String {
        self.note("normalize");
        self.inner.normalize(path)
    }

    fn is_absolute(&self, path: &str) -> bool {
        self.note("is_absolute");
        self.inner.is_absolute(path)
    }

    fn parse(&self, path: &str) -> Parsed {
        self.note("parse");
        self.inner.parse(path)
    }

    fn format(&self, parsed: &Parsed) -> String {
        self.note("format");
        self.inner.format(parsed)
    }

    fn relative(&self, from: &str, to: &str) -> String {
        self.note("relative");
        self.inner.relative(from, to)
    }

    fn from_file_url(&self, url: &str) -> Result<String, String> {
        self.note("from_file_url");
        self.inner.from_file_url(url)
    }
}
