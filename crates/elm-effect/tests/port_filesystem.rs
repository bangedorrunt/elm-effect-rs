//! Ports the non-handle slice of
//! `packages/effect/test/FileSystem.test-utils.ts` to Rust's FileSystem trait:
//! readFile, temp dirs/files (+scoped cleanup), access, truncate, write flags,
//! and non-clobbering copy. File-handle cursor rows need descriptor machinery
//! this port does not have (see the ledger).

use elm_effect::filesystem::{
    FileSystem, ScopedTempDir, StdFileSystem, make_temp_dir, make_temp_file_in,
};

#[test]
fn filesystem_read_file_reads_text() {
    let dir = std::env::temp_dir()
        .join("elm_effect_port_fs_read")
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(&dir).unwrap();
    let path = std::path::Path::new(&dir).join("text.txt");
    std::fs::write(&path, b"lorem ipsum dolar sit amet\n").unwrap();
    let fs = StdFileSystem;
    let bytes = fs.read_file(path.to_str().unwrap()).expect("read file");
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(text.trim(), "lorem ipsum dolar sit amet");
    std::fs::remove_file(&path).ok();
    std::fs::remove_dir(&dir).ok();
}

#[test]
fn filesystem_truncate_adjusts_content() {
    let dir = std::env::temp_dir()
        .join("elm_effect_port_fs_trunc")
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(&dir).unwrap();
    let path = std::path::Path::new(&dir).join("file.txt");
    std::fs::write(&path, b"hello world").unwrap();
    let fs = StdFileSystem;
    let before = fs.read_text(path.to_str().unwrap()).unwrap();
    assert_eq!(before, "hello world");
    fs.truncate(path.to_str().unwrap(), 5).expect("truncate 5");
    let truncated = fs.read_text(path.to_str().unwrap()).unwrap();
    assert_eq!(truncated, "hello");
    fs.truncate(path.to_str().unwrap(), 0)
        .expect("truncate empty");
    let after = fs.read_text(path.to_str().unwrap()).unwrap();
    assert_eq!(after, "");
    std::fs::remove_file(&path).ok();
    std::fs::remove_dir(&dir).ok();
}

#[test]
fn filesystem_make_temp_directory_persists() {
    // FileSystem.test-utils.ts:29 — makeTempDirectory survives its scope.
    let fs = StdFileSystem;
    let dir = make_temp_dir().expect("make temp dir");
    let info = fs.stat(&dir).expect("stat temp dir");
    assert!(info.is_dir);
    std::fs::remove_dir(&dir).ok();
    assert!(!fs.exists(&dir));
}

#[test]
fn filesystem_scoped_temp_directory_cleans_up() {
    // FileSystem.test-utils.ts:42 — the scoped directory is gone after drop.
    let fs = StdFileSystem;
    let path = {
        let scope = ScopedTempDir::new().expect("scoped temp dir");
        let path = scope.path().to_owned();
        assert!(fs.stat(&path).expect("stat").is_dir);
        path
    };
    assert!(!fs.exists(&path));
}

#[test]
fn filesystem_access_on_writable_directory() {
    // FileSystem.test-utils.ts:57 — access succeeds on a writable directory.
    let fs = StdFileSystem;
    let scope = ScopedTempDir::new().expect("scoped temp dir");
    fs.access(scope.path()).expect("access writable dir");
    assert!(fs.access("/nonexistent-elm-effect-dir-xyz").is_err());
}

#[test]
fn filesystem_scoped_temp_file_cleans_up() {
    // FileSystem.test-utils.ts:69 — the scoped temp file (and dir) vanish.
    let fs = StdFileSystem;
    let (dir, file) = {
        let scope = ScopedTempDir::new().expect("scoped temp dir");
        let file = make_temp_file_in(scope.path()).expect("temp file");
        assert!(fs.exists(&file));
        (scope.path().to_owned(), file)
    };
    assert!(!fs.exists(&file));
    assert!(!fs.exists(&dir));
}

#[test]
fn filesystem_write_r_plus_overwrites_without_truncating() {
    // FileSystem.test-utils.ts:114 — r+ replaces the prefix, keeps the tail.
    let fs = StdFileSystem;
    let scope = ScopedTempDir::new().expect("scoped temp dir");
    let path = format!("{}/a.txt", scope.path());
    fs.write_file(&path, b"abcdef").unwrap();
    fs.write_file_with_flag(&path, b"xy", "r+").unwrap();
    assert_eq!(fs.read_text(&path).unwrap(), "xycdef");
}

#[test]
fn filesystem_write_empty_data_honors_the_flag() {
    // FileSystem.test-utils.ts:125 — empty write truncates, empty r+ is a no-op.
    let fs = StdFileSystem;
    let scope = ScopedTempDir::new().expect("scoped temp dir");
    let path = format!("{}/a.txt", scope.path());
    fs.write_file(&path, b"abc").unwrap();
    fs.write_file(&path, b"").unwrap();
    assert_eq!(fs.read_text(&path).unwrap(), "");
    fs.write_file(&path, b"abc").unwrap();
    fs.write_file_with_flag(&path, b"", "r+").unwrap();
    assert_eq!(fs.read_text(&path).unwrap(), "abc");
}

#[test]
fn filesystem_write_r_rejects_writes() {
    // FileSystem.test-utils.ts:139 — r-flag writes fail as System (not BadArgument).
    let fs = StdFileSystem;
    let scope = ScopedTempDir::new().expect("scoped temp dir");
    let path = format!("{}/a.txt", scope.path());
    let err = fs
        .write_file_with_flag(&path, b"data", "r")
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(matches!(err, elm_effect::filesystem::PlatformError::System { .. }), "{msg}");
    assert!(msg.contains("writeFile") && msg.contains(&path), "{msg}");
}

#[test]
fn filesystem_write_a_appends() {
    // FileSystem.test-utils.ts:152 — the a flag appends.
    let fs = StdFileSystem;
    let scope = ScopedTempDir::new().expect("scoped temp dir");
    let path = format!("{}/a.txt", scope.path());
    fs.write_file(&path, b"abc").unwrap();
    fs.write_file_with_flag(&path, b"def", "a").unwrap();
    assert_eq!(fs.read_text(&path).unwrap(), "abcdef");
}

#[test]
fn filesystem_write_wx_exclusively_creates() {
    // FileSystem.test-utils.ts:163 — wx creates once, refuses twice.
    let fs = StdFileSystem;
    let scope = ScopedTempDir::new().expect("scoped temp dir");
    let path = format!("{}/file.txt", scope.path());
    fs.write_file_with_flag(&path, b"first", "wx").unwrap();
    assert!(fs.write_file_with_flag(&path, b"second", "wx").is_err());
    assert_eq!(fs.read_text(&path).unwrap(), "first");
}

#[test]
fn filesystem_copy_without_overwrite_preserves_destination() {
    // FileSystem.test-utils.ts:175 — no clobber; AlreadyExists tag inexpressible here.
    let fs = StdFileSystem;
    let scope = ScopedTempDir::new().expect("scoped temp dir");
    let src = format!("{}/source.txt", scope.path());
    let dst = format!("{}/destination.txt", scope.path());
    fs.write_file(&src, b"source").unwrap();
    fs.write_file(&dst, b"destination").unwrap();
    fs.copy_with_overwrite(&src, &dst, false).unwrap();
    assert_eq!(fs.read_text(&src).unwrap(), "source");
    assert_eq!(fs.read_text(&dst).unwrap(), "destination");
}
