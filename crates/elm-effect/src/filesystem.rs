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
        String::from_utf8(bytes).map_err(|_| {
            bad_argument("FileSystem", "read_text", "file is not valid UTF-8")
        })
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
        std::fs::write(path, bytes).map_err(|e| system_error("FileSystem", "writeFile", Some(path), &e))
    }

    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn stat(&self, path: &str) -> Result<FileInfo, PlatformError> {
        let meta =
            std::fs::metadata(path).map_err(|e| system_error("FileSystem", "stat", Some(path), &e))?;
        Ok(FileInfo { len: meta.len(), is_dir: meta.is_dir(), is_file: meta.is_file() })
    }

    fn make_dir(&self, path: &str) -> Result<(), PlatformError> {
        std::fs::create_dir_all(path)
            .map_err(|e| system_error("FileSystem", "makeDir", Some(path), &e))
    }

    fn read_dir(&self, path: &str) -> Result<Vec<String>, PlatformError> {
        let entries =
            std::fs::read_dir(path).map_err(|e| system_error("FileSystem", "readDir", Some(path), &e))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|e| system_error("FileSystem", "readDir", Some(path), &e))?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn remove(&self, path: &str) -> Result<(), PlatformError> {
        let meta =
            std::fs::symlink_metadata(path).map_err(|e| system_error("FileSystem", "remove", Some(path), &e))?;
        if meta.is_dir() {
            std::fs::remove_dir(path).map_err(|e| system_error("FileSystem", "remove", Some(path), &e))
        } else {
            std::fs::remove_file(path).map_err(|e| system_error("FileSystem", "remove", Some(path), &e))
        }
    }

    fn truncate(&self, path: &str, len: u64) -> Result<(), PlatformError> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| system_error("FileSystem", "truncate", Some(path), &e))?;
        file.set_len(len).map_err(|e| system_error("FileSystem", "truncate", Some(path), &e))
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
        self.tree.lock().unwrap().insert(Self::norm(path), FakeNode::File(bytes.to_vec()));
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        self.note("exists");
        self.tree.lock().unwrap().contains_key(&Self::norm(path))
    }

    fn stat(&self, path: &str) -> Result<FileInfo, PlatformError> {
        self.note("stat");
        match self.tree.lock().unwrap().get(&Self::norm(path)) {
            Some(FakeNode::File(bytes)) => {
                Ok(FileInfo { len: bytes.len() as u64, is_dir: false, is_file: true })
            }
            Some(FakeNode::Dir) => Ok(FileInfo { len: 0, is_dir: true, is_file: false }),
            None => Err(bad_argument("FileSystem", "stat", "no such path")),
        }
    }

    fn make_dir(&self, path: &str) -> Result<(), PlatformError> {
        self.note("makeDir");
        self.tree.lock().unwrap().insert(Self::norm(path), FakeNode::Dir);
        Ok(())
    }

    fn read_dir(&self, path: &str) -> Result<Vec<String>, PlatformError> {
        self.note("readDir");
        let base = Self::norm(path);
        let tree = self.tree.lock().unwrap();
        if !matches!(tree.get(&base), Some(FakeNode::Dir)) {
            return Err(bad_argument("FileSystem", "readDir", "not a directory"));
        }
        let prefix = if base == "/" { "/".to_string() } else { format!("{base}/") };
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
}
