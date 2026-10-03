//! Law L5 (tags part): service tags are zero-sized types with a string id;
//! providing removes the tag at the type level. A missing service is a
//! compile error, never a runtime lookup.

#[cfg(feature = "tokio")]
use elm_effect::Runtime;
use elm_effect::{Cons, Effect, Requirement, ServiceTag, use_service};
use std::convert::Infallible;

macro_rules! svc {
    ($Tag:ident, $Svc:ident, $id:literal) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $Svc(pub String);
        pub struct $Tag;
        impl ServiceTag for $Tag {
            type Service = $Svc;
            const ID: &'static str = $id;
        }
    };
}

svc!(Database, Db, "myapp/db/Database");
svc!(Clock, Tick, "myapp/clock/Clock");
svc!(S03, A03, "test/s03");
svc!(S04, A04, "test/s04");
svc!(S05, A05, "test/s05");
svc!(S06, A06, "test/s06");
svc!(S07, A07, "test/s07");
svc!(S08, A08, "test/s08");
svc!(S09, A09, "test/s09");
svc!(S10, A10, "test/s10");
svc!(S11, A11, "test/s11");
svc!(S12, A12, "test/s12");
svc!(S13, A13, "test/s13");
svc!(S14, A14, "test/s14");
svc!(S15, A15, "test/s15");
svc!(S16, A16, "test/s16");

fn db_url() -> Db {
    Db("postgres://localhost/app".to_string())
}

#[test]
#[cfg(feature = "tokio")]
fn tagged_service_resolves() {
    let rt = Runtime::new();
    let e: Effect<String, Infallible, Database> =
        use_service::<Database>().map(|db: Db| db.0.clone());
    let provided: Effect<String, Infallible, ()> = e.provide::<Database, ()>(db_url());
    let exit = rt.spawn_effect(provided).join();
    assert_eq!(
        exit,
        elm_effect::Exit::Success("postgres://localhost/app".to_string())
    );
}

#[test]
fn provide_removes_requirement() {
    // Compile evidence: each provide strips the head tag from the tuple.
    let e: Effect<String, String, (Database, Clock)> = Effect::succeed("x".to_string());
    let e2: Effect<String, String, (Clock,)> = e.provide::<Database, (Clock,)>(db_url());
    let e3: Effect<String, String, ()> = e2.provide::<Clock, ()>(Tick("t".to_string()));
    drop(e3);

    // Single-tag requirement strips to ().
    let s: Effect<i32, String, Database> = Effect::succeed(1);
    let _stripped: Effect<i32, String, ()> = s.provide::<Database, ()>(db_url());
}

#[test]
#[cfg(feature = "tokio")]
fn provided_values_reach_use_nodes() {
    // Provided values reach Use nodes through flat_map chains.
    let rt = Runtime::new();
    let chained: Effect<String, Infallible, Database> =
        Effect::<String, Infallible, Database>::succeed("ignored".to_string())
            .flat_map(|_| use_service::<Database>().map(|db: Db| db.0.clone()));
    let run: Effect<String, Infallible, ()> =
        chained.provide::<Database, ()>(Db("chained".to_string()));
    assert_eq!(
        rt.spawn_effect(run).join(),
        elm_effect::Exit::Success("chained".to_string())
    );
}

#[test]
fn provide_merge_removes_two_at_once() {
    let e: Effect<String, String, (Database, Clock)> = Effect::succeed("x".to_string());
    let merged: Effect<String, String, ()> = e.provide_merge(db_url(), Tick("t".to_string()));
    drop(merged);
}

#[test]
#[cfg(feature = "tokio")]
fn sixteen_services() {
    type R16 = (
        Database,
        Clock,
        S03,
        S04,
        S05,
        S06,
        S07,
        S08,
        S09,
        S10,
        S11,
        S12,
        S13,
        S14,
        S15,
        S16,
    );
    // The 16-tuple: every tag strips in order, ending at ().
    let e: Effect<i32, String, R16> = Effect::succeed(16);
    let e = e.provide::<Database, (
        Clock,
        S03,
        S04,
        S05,
        S06,
        S07,
        S08,
        S09,
        S10,
        S11,
        S12,
        S13,
        S14,
        S15,
        S16,
    )>(db_url());
    let e = e.provide::<Clock, (
        S03,
        S04,
        S05,
        S06,
        S07,
        S08,
        S09,
        S10,
        S11,
        S12,
        S13,
        S14,
        S15,
        S16,
    )>(Tick("t".to_string()));
    let e: Effect<i32, String, ()> = e
        .provide::<S03, (
            S04,
            S05,
            S06,
            S07,
            S08,
            S09,
            S10,
            S11,
            S12,
            S13,
            S14,
            S15,
            S16,
        )>(A03("3".to_string()))
        .provide::<S04, (S05, S06, S07, S08, S09, S10, S11, S12, S13, S14, S15, S16)>(A04(
            "4".to_string()
        ))
        .provide::<S05, (S06, S07, S08, S09, S10, S11, S12, S13, S14, S15, S16)>(A05(
            "5".to_string()
        ))
        .provide::<S06, (S07, S08, S09, S10, S11, S12, S13, S14, S15, S16)>(A06("6".to_string()))
        .provide::<S07, (S08, S09, S10, S11, S12, S13, S14, S15, S16)>(A07("7".to_string()))
        .provide::<S08, (S09, S10, S11, S12, S13, S14, S15, S16)>(A08("8".to_string()))
        .provide::<S09, (S10, S11, S12, S13, S14, S15, S16)>(A09("9".to_string()))
        .provide::<S10, (S11, S12, S13, S14, S15, S16)>(A10("10".to_string()))
        .provide::<S11, (S12, S13, S14, S15, S16)>(A11("11".to_string()))
        .provide::<S12, (S13, S14, S15, S16)>(A12("12".to_string()))
        .provide::<S13, (S14, S15, S16)>(A13("13".to_string()))
        .provide::<S14, (S15, S16)>(A14("14".to_string()))
        .provide::<S15, (S16,)>(A15("15".to_string()))
        .provide::<S16, ()>(A16("16".to_string()));
    let rt = Runtime::new();
    assert_eq!(rt.spawn_effect(e).join(), elm_effect::Exit::Success(16));

    // Beyond 16, Cons is the escape: head removal works the same.
    let c: Effect<i32, String, Cons<Database, ()>> = Effect::succeed(17);
    let c: Effect<i32, String, ()> = c.provide::<Database, ()>(db_url());
    assert_eq!(rt.spawn_effect(c).join(), elm_effect::Exit::Success(17));

    // Requirement is a closed world: () needs nothing, tags need themselves.
    fn needs_nothing<R: Requirement>() {}
    needs_nothing::<()>();
}

#[test]
#[cfg(feature = "tokio")]
fn effect_macro_expands_to_flat_map() {
    let rt = Runtime::new();
    let e = elm_effect::effect! {
        let a = Effect::<i32, Infallible>::succeed(20);
        let b = Effect::<i32, Infallible>::succeed(22);
        Effect::<i32, Infallible>::succeed(a + b)
    };
    assert_eq!(rt.spawn_effect(e).join(), elm_effect::Exit::Success(42));

    // A typed failure short-circuits the rest: `b` never runs.
    let e2: Effect<i32, String, ()> = elm_effect::effect! {
        let a = Effect::<i32, String>::fail("short".to_string());
        let b = Effect::succeed(a + 1);
        Effect::succeed(b)
    };
    let exit = rt.spawn_effect(e2).join();
    assert_eq!(
        exit,
        elm_effect::Exit::Failure(elm_effect::Cause::fail("short".to_string()))
    );
}

/// Trybuild-style negative test without new dependencies: build a scratch
/// crate depending on this one (path dependency, offline) and assert
/// `cargo check` FAILS on the missing tag. The scratch project lives in the
/// temp dir so no target-dir layout is assumed.
#[test]
fn missing_service_does_not_compile() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let fixture = format!("{manifest}/tests/ui/missing_service_fail.rs");
    assert!(
        std::path::Path::new(&fixture).exists(),
        "fixture missing: {fixture}"
    );
    let krate = std::path::Path::new(&manifest)
        .canonicalize()
        .expect("canonicalize");
    let krate = krate.display().to_string();

    let dir = std::env::temp_dir().join(format!("elm-effect-uifail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("scratch src");
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"uifail\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             [dependencies]\nelm-effect = {{ path = \"{krate}\" }}\n\
             [workspace]\n"
        ),
    )
    .expect("scratch manifest");
    std::fs::copy(&fixture, dir.join("src/lib.rs")).expect("scratch lib");

    let out = std::process::Command::new("cargo")
        .args([
            "check",
            "--offline",
            "--manifest-path",
            dir.join("Cargo.toml").to_str().expect("utf8"),
        ])
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .expect("spawn cargo check");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.status.success(),
        "fixture must NOT compile: a missing service has to be a compile error"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Database"),
        "error must name the missing tag; got:\n{stderr}"
    );
}
