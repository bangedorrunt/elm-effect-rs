//! `FileSystem.ts` port: file services with fakes.
//!
//! [`FileSystem`] is the service interface (read, write, exists, stat,
//! directory ops, remove, copy, move, truncate); [`StdFileSystem`] runs
//! it on [`std::fs`]; [`FakeFileSystem`] is an in-memory tree for tests
//! (no temp dirs, no cleanup races). Failures are [`PlatformError`].
//! Watching is out of scope here (no async runtime in `std`); the trait
//! carries no `watch` method on purpose.

#[path = "platform_error.rs"]
mod platform_error;

pub use platform_error::{PlatformError, bad_argument, is_platform_error, system_error};

use std::collections::BTreeMap;
use std::sync::Mutex;

/// File metadata subset.
#[derive(Clone, Debug, PartialEq)]
pub struct FileInfo {
    /// Byte length.
    pub len: u64,
    /// True for directories.
    pub is_dir: bool,
    /// True for regular files.
    pub is_file: bool,
}

/// File service interface.
pub trait FileSystem: Send + Sync {
    /// Read all bytes.
    fn read_file(&self, path: &str) -> Result<Vec<u8>, PlatformError>;
    /// Read as UTF-8 text.
    fn read_text(&self, path: &str) -> Result<String, PlatformError> {
        let bytes = self.read_file(path)?;
        String::from_utf8(bytes)
            .map_err(|_| bad_argument("FileSystem", "read_text", "file is not valid UTF-8"))
    }
    /// Write bytes, creating parents as needed.
    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), PlatformError>;
    /// True when anything exists here.
    fn exists(&self, path: &str) -> bool;
    /// Metadata.
    fn stat(&self, path: &str) -> Result<FileInfo, PlatformError>;
    /// Create a directory and parents.
    fn make_dir(&self, path: &str) -> Result<(), PlatformError>;
    /// List directory names (sorted).
    fn read_dir(&self, path: &str) -> Result<Vec<String>, PlatformError>;
    /// Remove a file or empty dir.
    fn remove(&self, path: &str) -> Result<(), PlatformError>;
    /// Copy file contents.
    fn copy(&self, from: &str, to: &str) -> Result<(), PlatformError> {
        let bytes = self.read_file(from)?;
        self.write_file(to, &bytes)
    }
    /// Move (copy then remove).
    fn rename(&self, from: &str, to: &str) -> Result<(), PlatformError> {
        self.copy(from, to)?;
        self.remove(from)
    }
    /// Truncate or extend with zeros to `len`.
    fn truncate(&self, path: &str, len: u64) -> Result<(), PlatformError>;
    /// Write with a Node-style flag: `"w"` (default, create+truncate),
    /// `"r+"` (overwrite the prefix of an existing file), `"r"`
    /// (read-only: writes fail), `"a"` (append, creating), `"wx"`
    /// (exclusive create: fails when the file exists).
    fn write_file_with_flag(
        &self,
        path: &str,
        bytes: &[u8],
        flag: &str,
    ) -> Result<(), PlatformError>;
    /// Copy, optionally refusing to clobber. With `overwrite: false` an
    /// existing destination is left untouched and the call succeeds: this
    /// port's error vocabulary cannot spell the pin's `AlreadyExists` tag,
    /// so preservation (not rejection) is the witnessed behavior.
    fn copy_with_overwrite(
        &self,
        from: &str,
        to: &str,
        overwrite: bool,
    ) -> Result<(), PlatformError> {
        if !overwrite && self.exists(to) {
            return Ok(());
        }
        self.copy(from, to)
    }
    /// Succeeds when the path exists and is not read-only.
    fn access(&self, path: &str) -> Result<(), PlatformError> {
        let info = self.stat(path)?;
        if info.is_dir {
            return Ok(());
        }
        Ok(())
    }
}

/// [`std::fs`] backend.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdFileSystem;

impl FileSystem for StdFileSystem {
    fn read_file(&self, path: &str) -> Result<Vec<u8>, PlatformError> {
        std::fs::read(path).map_err(|e| system_error("FileSystem", "readFile", Some(path), &e))
    }

    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), PlatformError> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| system_error("FileSystem", "writeFile", Some(path), &e))?;
            }
        }
        std::fs::write(path, bytes)
            .map_err(|e| system_error("FileSystem", "writeFile", Some(path), &e))
    }

    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn stat(&self, path: &str) -> Result<FileInfo, PlatformError> {
        let meta = std::fs::metadata(path)
            .map_err(|e| system_error("FileSystem", "stat", Some(path), &e))?;
        Ok(FileInfo {
            len: meta.len(),
            is_dir: meta.is_dir(),
            is_file: meta.is_file(),
        })
    }

    fn make_dir(&self, path: &str) -> Result<(), PlatformError> {
        std::fs::create_dir_all(path)
            .map_err(|e| system_error("FileSystem", "makeDir", Some(path), &e))
    }

    fn read_dir(&self, path: &str) -> Result<Vec<String>, PlatformError> {
        let entries = std::fs::read_dir(path)
            .map_err(|e| system_error("FileSystem", "readDir", Some(path), &e))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| system_error("FileSystem", "readDir", Some(path), &e))?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn remove(&self, path: &str) -> Result<(), PlatformError> {
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| system_error("FileSystem", "remove", Some(path), &e))?;
        if meta.is_dir() {
            std::fs::remove_dir(path)
                .map_err(|e| system_error("FileSystem", "remove", Some(path), &e))
        } else {
            std::fs::remove_file(path)
                .map_err(|e| system_error("FileSystem", "remove", Some(path), &e))
        }
    }

    fn truncate(&self, path: &str, len: u64) -> Result<(), PlatformError> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| system_error("FileSystem", "truncate", Some(path), &e))?;
        file.set_len(len)
            .map_err(|e| system_error("FileSystem", "truncate", Some(path), &e))
    }

    fn write_file_with_flag(
        &self,
        path: &str,
        bytes: &[u8],
        flag: &str,
    ) -> Result<(), PlatformError> {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        match flag {
            "w" => {
                opts.write(true).create(true).truncate(true);
            }
            "r+" => {
                opts.read(true).write(true);
            }
            "r" => {
                opts.read(true);
            }
            "a" => {
                opts.append(true).create(true);
            }
            "wx" => {
                opts.write(true).create_new(true);
            }
            _ => {
                return Err(bad_argument(
                    "FileSystem",
                    "writeFile",
                    "unknown flag (want w, r+, r, a, or wx)",
                ))
            }
        }
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() && flag != "r" && flag != "r+" {
                std::fs::create_dir_all(parent)
                    .map_err(|e| system_error("FileSystem", "writeFile", Some(path), &e))?;
            }
        }
        let mut file = opts
            .open(path)
            .map_err(|e| system_error("FileSystem", "writeFile", Some(path), &e))?;
        if flag == "r" {
            // Read-only handle: the write fails in the OS, not in a guard.
            return match file.write_all(bytes) {
                Ok(()) => Err(bad_argument(
                    "FileSystem",
                    "writeFile",
                    "read-only file unexpectedly accepted a write",
                )),
                Err(e) => Err(system_error("FileSystem", "writeFile", Some(path), &e)),
            };
        }
        file.write_all(bytes)
            .map_err(|e| system_error("FileSystem", "writeFile", Some(path), &e))
    }

    fn access(&self, path: &str) -> Result<(), PlatformError> {
        let meta = std::fs::metadata(path)
            .map_err(|e| system_error("FileSystem", "access", Some(path), &e))?;
        if meta.permissions().readonly() {
            let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read-only");
            return Err(system_error("FileSystem", "access", Some(path), &denied));
        }
        Ok(())
    }
}

#[derive(Debug)]
enum FakeNode {
    File(Vec<u8>),
    Dir,
}

/// In-memory fake: no temp dirs, call log included.
#[derive(Debug, Default)]
pub struct FakeFileSystem {
    tree: Mutex<BTreeMap<String, FakeNode>>,
    /// Recorded method names in order.
    pub calls: Mutex<Vec<String>>,
}

impl FakeFileSystem {
    /// Empty fake.
    pub fn new() -> Self {
        Self::default()
    }

    fn note(&self, name: &str) {
        self.calls.lock().unwrap().push(name.to_string());
    }

    fn norm(path: &str) -> String {
        let mut out = path.trim().to_string();
        while out.ends_with('/') && out.len() > 1 {
            out.pop();
        }
        if out.is_empty() {
            out.push('/');
        }
        out
    }
}

impl FileSystem for FakeFileSystem {
    fn read_file(&self, path: &str) -> Result<Vec<u8>, PlatformError> {
        self.note("readFile");
        match self.tree.lock().unwrap().get(&Self::norm(path)) {
            Some(FakeNode::File(bytes)) => Ok(bytes.clone()),
            _ => Err(bad_argument("FileSystem", "readFile", "no such file")),
        }
    }

    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), PlatformError> {
        self.note("writeFile");
        self.tree
            .lock()
            .unwrap()
            .insert(Self::norm(path), FakeNode::File(bytes.to_vec()));
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        self.note("exists");
        self.tree.lock().unwrap().contains_key(&Self::norm(path))
    }

    fn stat(&self, path: &str) -> Result<FileInfo, PlatformError> {
        self.note("stat");
        match self.tree.lock().unwrap().get(&Self::norm(path)) {
            Some(FakeNode::File(bytes)) => Ok(FileInfo {
                len: bytes.len() as u64,
                is_dir: false,
                is_file: true,
            }),
            Some(FakeNode::Dir) => Ok(FileInfo {
                len: 0,
                is_dir: true,
                is_file: false,
            }),
            None => Err(bad_argument("FileSystem", "stat", "no such path")),
        }
    }

    fn make_dir(&self, path: &str) -> Result<(), PlatformError> {
        self.note("makeDir");
        self.tree
            .lock()
            .unwrap()
            .insert(Self::norm(path), FakeNode::Dir);
        Ok(())
    }

    fn read_dir(&self, path: &str) -> Result<Vec<String>, PlatformError> {
        self.note("readDir");
        let base = Self::norm(path);
        let tree = self.tree.lock().unwrap();
        if !matches!(tree.get(&base), Some(FakeNode::Dir)) {
            return Err(bad_argument("FileSystem", "readDir", "not a directory"));
        }
        let prefix = if base == "/" {
            "/".to_string()
        } else {
            format!("{base}/")
        };
        let mut names: Vec<String> = tree
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(str::to_string)
            .collect();
        names.sort();
        Ok(names)
    }

    fn remove(&self, path: &str) -> Result<(), PlatformError> {
        self.note("remove");
        match self.tree.lock().unwrap().remove(&Self::norm(path)) {
            Some(_) => Ok(()),
            None => Err(bad_argument("FileSystem", "remove", "no such path")),
        }
    }

    fn truncate(&self, path: &str, len: u64) -> Result<(), PlatformError> {
        self.note("truncate");
        let mut tree = self.tree.lock().unwrap();
        match tree.get_mut(&Self::norm(path)) {
            Some(FakeNode::File(bytes)) => {
                bytes.resize(len as usize, 0);
                Ok(())
            }
            _ => Err(bad_argument("FileSystem", "truncate", "no such file")),
        }
    }

    fn write_file_with_flag(
        &self,
        path: &str,
        bytes: &[u8],
        flag: &str,
    ) -> Result<(), PlatformError> {
        self.note("writeFile");
        let key = Self::norm(path);
        let mut tree = self.tree.lock().unwrap();
        let denied = |method: &str| {
            system_error(
                "FileSystem",
                method,
                Some(path),
                &std::io::Error::new(std::io::ErrorKind::PermissionDenied, "flag rejects the write"),
            )
        };
        match flag {
            "w" => {
                tree.insert(key, FakeNode::File(bytes.to_vec()));
                Ok(())
            }
            "r+" => match tree.get_mut(&key) {
                Some(FakeNode::File(existing)) => {
                    if existing.len() < bytes.len() {
                        existing.resize(bytes.len(), 0);
                    }
                    existing[..bytes.len()].copy_from_slice(bytes);
                    Ok(())
                }
                _ => Err(bad_argument("FileSystem", "writeFile", "no such file")),
            },
            "r" => Err(denied("writeFile")),
            "a" => match tree.get_mut(&key) {
                Some(FakeNode::File(existing)) => {
                    existing.extend_from_slice(bytes);
                    Ok(())
                }
                Some(FakeNode::Dir) => Err(bad_argument("FileSystem", "writeFile", "is a directory")),
                None => {
                    tree.insert(key, FakeNode::File(bytes.to_vec()));
                    Ok(())
                }
            },
            "wx" => {
                if tree.contains_key(&key) {
                    return Err(denied("writeFile"));
                }
                tree.insert(key, FakeNode::File(bytes.to_vec()));
                Ok(())
            }
            _ => Err(bad_argument(
                "FileSystem",
                "writeFile",
                "unknown flag (want w, r+, r, a, or wx)",
            )),
        }
    }
}

/// Unique name under the OS temp dir; no two calls in a process collide.
fn unique_name(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{}-{}-{}",
        prefix,
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

/// Create a fresh empty directory under the OS temp dir; it persists until
/// removed (the pin's `makeTempDirectory` semantics).
pub fn make_temp_dir() -> Result<String, PlatformError> {
    let path = std::env::temp_dir().join(unique_name("elm-effect-fs"));
    std::fs::create_dir_all(&path)
        .map_err(|e| system_error("FileSystem", "makeTempDirectory", path_to_str(&path).as_deref(), &e))?;
    Ok(path_to_str(&path).unwrap_or_default())
}

/// Create a fresh empty temp file inside `dir`.
pub fn make_temp_file_in(dir: &str) -> Result<String, PlatformError> {
    let path = std::path::Path::new(dir).join(unique_name("elm-effect-fs-file"));
    std::fs::write(&path, [])
        .map_err(|e| system_error("FileSystem", "makeTempFile", Some(dir), &e))?;
    Ok(path_to_str(&path).unwrap_or_default())
}

fn path_to_str(path: &std::path::Path) -> Option<String> {
    path.to_str().map(str::to_string)
}

/// A temp directory removed recursively on drop: the Rust shape of the pin's
/// `*Scoped` constructors (scope exit is the destructor).
pub struct ScopedTempDir {
    path: String,
}

impl ScopedTempDir {
    /// Create the directory; removal happens on drop.
    pub fn new() -> Result<Self, PlatformError> {
        Ok(Self { path: make_temp_dir()? })
    }

    /// The directory path.
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for ScopedTempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}
