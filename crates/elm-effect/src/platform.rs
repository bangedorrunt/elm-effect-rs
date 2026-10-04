//! Platform substitutes: `FileSystem`, `Path`, and platform errors.
//!
//! `std::fs` lives behind the [`FileSystem`] service so domain code never
//! touches the ambient filesystem; tests pass [`fakes`](FileSystem::fake).
//! [`Path`] is pure lexical math over [`std::path`]. Failures are typed
//! [`SystemError`]s with the pin's normalized tags.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

// PlatformError

/// Normalized failure tags, per the pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SystemErrorTag {
    /// Bad argument.
    BadArgument,
    /// Data failed to parse or validate.
    InvalidData,
    /// No such file or entry.
    NotFound,
    /// Permission denied.
    PermissionDenied,
    /// Timed out.
    TimedOut,
    /// Unexpected end of input.
    UnexpectedEof,
    /// Anything else.
    Unknown,
    /// Would block.
    WouldBlock,
    /// Wrote zero bytes.
    WriteZero,
}

/// A platform failure: normalized tag plus where it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemError {
    /// Normalized reason.
    pub tag: SystemErrorTag,
    /// Module that failed (e.g. `"FileSystem"`).
    pub module: String,
    /// Method that failed (e.g. `"readFileString"`).
    pub method: String,
    /// The path involved, if any.
    pub path: Option<String>,
    /// Human detail.
    pub message: String,
}

impl SystemError {
    /// Build from parts.
    pub fn new(
        tag: SystemErrorTag,
        module: impl Into<String>,
        method: impl Into<String>,
        path: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            tag,
            module: module.into(),
            method: method.into(),
            path,
            message: message.into(),
        }
    }

    /// Normalize a [`std::io::Error`] for a module method and path.
    pub fn from_io(error: std::io::Error, module: &str, method: &str, path: &str) -> Self {
        use std::io::ErrorKind as K;
        let tag = match error.kind() {
            K::NotFound => SystemErrorTag::NotFound,
            K::PermissionDenied => SystemErrorTag::PermissionDenied,
            K::TimedOut => SystemErrorTag::TimedOut,
            K::UnexpectedEof => SystemErrorTag::UnexpectedEof,
            K::WouldBlock => SystemErrorTag::WouldBlock,
            K::WriteZero => SystemErrorTag::WriteZero,
            K::InvalidData | K::InvalidInput => SystemErrorTag::InvalidData,
            _ => SystemErrorTag::Unknown,
        };
        Self::new(
            tag,
            module,
            method,
            Some(path.to_string()),
            error.to_string(),
        )
    }

    /// Whether this names a missing entry.
    pub fn is_not_found(&self) -> bool {
        self.tag == SystemErrorTag::NotFound
    }
}

impl std::fmt::Display for SystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}: {}.{}{}",
            self.tag, self.module, self.method, self.message
        )
    }
}

impl std::error::Error for SystemError {}

// Path

/// Pure lexical path math over [`std::path`]. No I/O, no symlinks resolved.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathSvc;

impl PathSvc {
    /// Join segments with `/`, then normalize.
    pub fn join(&self, parts: &[&str]) -> String {
        self.normalize(&parts.join("/"))
    }

    /// Lexically normalize: collapse `.`, resolve `..`, drop empties,
    /// preserve a leading `/` (absolute) or lack of one (relative).
    pub fn normalize(&self, path: &str) -> String {
        let absolute = path.starts_with('/');
        let mut out: Vec<&str> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if out.pop().is_none() && !absolute {
                        out.push("..");
                    }
                }
                _ => out.push(part),
            }
        }
        let joined = out.join("/");
        if absolute {
            format!("/{joined}")
        } else if joined.is_empty() {
            ".".to_string()
        } else {
            joined
        }
    }

    /// Final component, minus `suffix` when present.
    pub fn basename(&self, path: &str, suffix: &str) -> String {
        let base = path.rsplit('/').next().unwrap_or(path).to_string();
        if !suffix.is_empty() && base.ends_with(suffix) {
            base[..base.len() - suffix.len()].to_string()
        } else {
            base
        }
    }

    /// Everything but the final component (`/` for bare roots, `.` for bare names).
    pub fn dirname(&self, path: &str) -> String {
        match path.rfind('/') {
            None => ".".to_string(),
            Some(0) => "/".to_string(),
            Some(i) => {
                let dir = &path[..i];
                if dir.is_empty() {
                    "/".to_string()
                } else {
                    dir.to_string()
                }
            }
        }
    }

    /// The dot extension, including the dot ("" when none).
    pub fn extname(&self, path: &str) -> String {
        let base = path.rsplit('/').next().unwrap_or(path);
        match base.rfind('.') {
            Some(0) | None => "".to_string(),
            Some(i) => base[i..].to_string(),
        }
    }

    /// Whether the path is absolute.
    pub fn is_absolute(&self, path: &str) -> bool {
        path.starts_with('/')
    }
}

/// The `Path` service tag.
pub struct PathTag;

impl elm_effect::ServiceTag for PathTag {
    type Service = PathSvc;
    const ID: &'static str = "elm-effect/path";
}

// FileSystem

/// File metadata: size plus kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    /// Byte size.
    pub size: u64,
    /// True for directories.
    pub is_dir: bool,
}

#[derive(Debug, Default, Clone)]
struct FakeState {
    files: HashMap<String, Vec<u8>>,
    dirs: HashSet<String>,
}

#[derive(Debug, Clone)]
enum Backend {
    Std,
    Fake(Arc<Mutex<FakeState>>),
}

/// The filesystem service. [`FileSystem::fake`] builds an in-memory backend
/// for tests; [`FileSystem::live`] uses [`std::fs`].
#[derive(Debug, Clone)]
pub struct FileSystem {
    backend: Backend,
}

/// The in-memory half of a fake: prefill and inspect around tests.
#[derive(Clone, Debug, Default)]
pub struct FakeFs {
    state: Arc<Mutex<FakeState>>,
}

impl FakeFs {
    /// Preload one file.
    pub fn put(&self, path: &str, content: &[u8]) {
        let mut state = self.state.lock().unwrap();
        state.files.insert(path.to_string(), content.to_vec());
    }

    /// Read back one file, if present.
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.state.lock().unwrap().files.get(path).cloned()
    }
}

impl FileSystem {
    /// The live backend: [`std::fs`].
    pub fn live() -> Self {
        Self {
            backend: Backend::Std,
        }
    }

    /// An in-memory backend plus its handle for prefilling and inspection.
    pub fn fake() -> (Self, FakeFs) {
        let state = Arc::new(Mutex::new(FakeState::default()));
        (
            Self {
                backend: Backend::Fake(Arc::clone(&state)),
            },
            FakeFs { state },
        )
    }

    /// Whether `path` exists.
    pub fn exists(&self, path: &str) -> bool {
        match &self.backend {
            Backend::Std => std::path::Path::new(path).exists(),
            Backend::Fake(state) => {
                let state = state.lock().unwrap();
                state.files.contains_key(path) || state.dirs.contains(path)
            }
        }
    }

    /// Read a whole file as a string.
    pub fn read_string(&self, path: &str) -> Result<String, SystemError> {
        match &self.backend {
            Backend::Std => std::fs::read_to_string(path)
                .map_err(|e| SystemError::from_io(e, "FileSystem", "readFileString", path)),
            Backend::Fake(state) => state
                .lock()
                .unwrap()
                .files
                .get(path)
                .and_then(|bytes| String::from_utf8(bytes.clone()).ok())
                .ok_or_else(|| {
                    SystemError::new(
                        SystemErrorTag::NotFound,
                        "FileSystem",
                        "readFileString",
                        Some(path.to_string()),
                        format!("no such file: {path}"),
                    )
                }),
        }
    }

    /// Write a whole file, creating parents on the live backend.
    pub fn write_string(&self, path: &str, content: &str) -> Result<(), SystemError> {
        match &self.backend {
            Backend::Std => {
                if let Some(parent) = std::path::Path::new(path).parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        SystemError::from_io(e, "FileSystem", "writeFileString", path)
                    })?;
                }
                std::fs::write(path, content)
                    .map_err(|e| SystemError::from_io(e, "FileSystem", "writeFileString", path))
            }
            Backend::Fake(state) => {
                state
                    .lock()
                    .unwrap()
                    .files
                    .insert(path.to_string(), content.as_bytes().to_vec());
                Ok(())
            }
        }
    }

    /// Create a directory (and parents).
    pub fn make_directory(&self, path: &str) -> Result<(), SystemError> {
        if path.is_empty() {
            return Err(SystemError::new(
                SystemErrorTag::BadArgument,
                "FileSystem",
                "makeDirectory",
                Some(path.to_string()),
                "empty path",
            ));
        }
        match &self.backend {
            Backend::Std => std::fs::create_dir_all(path)
                .map_err(|e| SystemError::from_io(e, "FileSystem", "makeDirectory", path)),
            Backend::Fake(state) => {
                state.lock().unwrap().dirs.insert(path.to_string());
                Ok(())
            }
        }
    }

    /// Remove a file or empty directory.
    pub fn remove(&self, path: &str) -> Result<(), SystemError> {
        match &self.backend {
            Backend::Std => {
                let fs_path = std::path::Path::new(path);
                if fs_path.is_dir() {
                    std::fs::remove_dir(path)
                } else {
                    std::fs::remove_file(path)
                }
                .map_err(|e| SystemError::from_io(e, "FileSystem", "remove", path))
            }
            Backend::Fake(state) => {
                let mut state = state.lock().unwrap();
                if state.files.remove(path).is_some() || state.dirs.remove(path) {
                    Ok(())
                } else {
                    Err(SystemError::new(
                        SystemErrorTag::NotFound,
                        "FileSystem",
                        "remove",
                        Some(path.to_string()),
                        format!("no such file: {path}"),
                    ))
                }
            }
        }
    }

    /// Size and kind of one entry.
    pub fn stat(&self, path: &str) -> Result<FileStat, SystemError> {
        match &self.backend {
            Backend::Std => std::fs::metadata(path)
                .map(|m| FileStat {
                    size: m.len(),
                    is_dir: m.is_dir(),
                })
                .map_err(|e| SystemError::from_io(e, "FileSystem", "stat", path)),
            Backend::Fake(state) => {
                let state = state.lock().unwrap();
                if let Some(bytes) = state.files.get(path) {
                    Ok(FileStat {
                        size: bytes.len() as u64,
                        is_dir: false,
                    })
                } else if state.dirs.contains(path) {
                    Ok(FileStat {
                        size: 0,
                        is_dir: true,
                    })
                } else {
                    Err(SystemError::new(
                        SystemErrorTag::NotFound,
                        "FileSystem",
                        "stat",
                        Some(path.to_string()),
                        format!("no such file: {path}"),
                    ))
                }
            }
        }
    }
}

/// The `FileSystem` service tag.
pub struct FileSystemTag;

impl elm_effect::ServiceTag for FileSystemTag {
    type Service = FileSystem;
    const ID: &'static str = "elm-effect/filesystem";
}
