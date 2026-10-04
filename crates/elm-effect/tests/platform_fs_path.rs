//! Platform substitutes: fakes under test, live `std` nowhere in domain code.

#[path = "../src/platform.rs"]
mod platform;

use elm_effect::ServiceTag as _;
use platform::{FakeFs, FileSystem, FileSystemTag, PathSvc, PathTag, SystemError, SystemErrorTag};

#[test]
fn fake_roundtrip_never_touches_disk() {
    let (fs, fake) = FileSystem::fake();
    assert!(!fs.exists("/cfg/app.json"));
    fake.put("/cfg/app.json", b"{\"env\":\"test\"}");
    assert!(fs.exists("/cfg/app.json"));
    assert_eq!(
        fs.read_string("/cfg/app.json").unwrap(),
        "{\"env\":\"test\"}"
    );
    fs.write_string("/cfg/app.json", "{\"env\":\"prod\"}")
        .unwrap();
    assert_eq!(fake.get("/cfg/app.json").unwrap(), b"{\"env\":\"prod\"}");
    assert_eq!(fs.stat("/cfg/app.json").unwrap().size, 14);
    assert!(fs.remove("/cfg/app.json").is_ok());
    assert!(!fs.exists("/cfg/app.json"));
}

#[test]
fn fake_missing_is_typed_not_found() {
    let (fs, _) = FileSystem::fake();
    let err = fs.read_string("/nope").expect_err("missing file");
    assert!(err.is_not_found());
    assert_eq!(err.tag, SystemErrorTag::NotFound);
    assert_eq!(err.module, "FileSystem");
    assert!(matches!(
        fs.remove("/nope"),
        Err(SystemError {
            tag: SystemErrorTag::NotFound,
            ..
        })
    ));
    assert!(fs.stat("/nope").is_err());
    fs.make_directory("/data").unwrap();
    assert!(fs.exists("/data"));
    assert_eq!(
        fs.make_directory("").expect_err("empty path"),
        SystemError::new(
            SystemErrorTag::BadArgument,
            "FileSystem",
            "makeDirectory",
            Some("".to_string()),
            "empty path",
        )
    );
    assert!(fs.stat("/data").unwrap().is_dir);
}

#[test]
fn path_ops_are_lexical() {
    let path = PathSvc;
    assert_eq!(path.join(&["/a", "b/", "./c", "../d"]), "/a/b/d");
    assert_eq!(path.normalize("/a//b/./c"), "/a/b/c");
    assert_eq!(path.normalize("a/../../b"), "../b");
    assert_eq!(path.basename("/a/b/file.txt", ""), "file.txt");
    assert_eq!(path.basename("/a/b/file.txt", ".txt"), "file");
    assert_eq!(path.dirname("/a/b/file.txt"), "/a/b");
    assert_eq!(path.dirname("file.txt"), ".");
    assert_eq!(path.extname("file.txt"), ".txt");
    assert_eq!(path.extname("noext"), "");
    assert!(path.is_absolute("/a"));
    assert!(!path.is_absolute("a"));
}

#[test]
fn services_provide_both_backends() {
    let (fs, _) = FileSystem::fake();
    let live = FileSystem::live();
    let dir = std::env::temp_dir().join(format!("elm-fs-{}", std::process::id()));
    let anchor = dir.join("probe.txt");
    let anchor_s = anchor.to_string_lossy().into_owned();
    live.make_directory(dir.to_str().unwrap()).unwrap();
    live.write_string(&anchor_s, "live").unwrap();
    assert_eq!(live.read_string(&anchor_s).unwrap(), "live");
    assert!(live.exists(&anchor_s));
    std::fs::remove_file(&anchor).unwrap();
    std::fs::remove_dir(&dir).unwrap();

    // Tags name stable services; fakes satisfy them.
    assert_eq!(FileSystemTag::ID, "elm-effect/filesystem");
    assert_eq!(PathTag::ID, "elm-effect/path");
    let _ = (fs, FakeFs::default());
}

#[test]
fn error_display_names_module_method() {
    let err = SystemError::new(
        SystemErrorTag::PermissionDenied,
        "FileSystem",
        "writeFileString",
        Some("/root/x".to_string()),
        "denied",
    );
    let shown = format!("{err}");
    assert!(shown.contains("FileSystem"));
    assert!(shown.contains("writeFileString"));
}
