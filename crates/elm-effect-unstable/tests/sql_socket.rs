//! SQL service + sockets: typed errors, journaled transactions, loopback I/O.

#[path = "../src/sql.rs"]
mod sql;
#[path = "../src/socket.rs"]
mod socket;

use socket::socket_pair;
use sql::{SqlClient, SqlError, SqlReason, classify_sqlite_error};

#[test]
fn sql_error_reasons_classify() {
    let unique = SqlError::new(
        SqlReason::UniqueViolation {
            constraint: Some("users_email_key".to_string()),
        },
        false,
        "duplicate key",
    );
    assert!(unique.is_unique_violation());
    assert!(!unique.retryable);

    assert!(matches!(
        classify_sqlite_error(Some("SQLITE_CONSTRAINT_UNIQUE"), "UNIQUE constraint failed: users.email"),
        SqlReason::UniqueViolation { constraint: Some(name) } if name == "users.email"
    ));
    assert!(matches!(
        classify_sqlite_error(Some("SQLITE_CONSTRAINT_CHECK"), "check failed"),
        SqlReason::ConstraintError { .. }
    ));
    assert_eq!(
        classify_sqlite_error(Some("SQLITE_CONSTRAINT_UNIQUE"), "no identifier here"),
        SqlReason::UniqueViolation { constraint: None }
    );
    assert_eq!(classify_sqlite_error(None, ""), SqlReason::Unknown);
    assert_eq!(classify_sqlite_error(Some(""), "x"), SqlReason::Unknown);
    for reason in [
        SqlReason::ForeignKeyViolation,
        SqlReason::SyntaxError,
        SqlReason::ConnectionLost,
    ] {
        let err = SqlError::new(reason, true, "boom");
        assert!(!err.is_unique_violation());
        assert!(err.retryable);
        assert!(err.to_string().contains("boom"));
    }
}

#[test]
fn failed_begin_never_rolls_back() {
    let db = SqlClient::fake();
    db.insert("t", vec!["seed".to_string()]);
    let err = SqlError::new(SqlReason::Unknown, false, "connect refused");
    let out = db.transaction(Err(err.clone()), || Ok::<u32, SqlError>(1));
    assert_eq!(out, Err(err));
    assert_eq!(db.scan("t"), vec![vec!["seed".to_string()]]);
}

#[test]
fn rollback_restores_on_effect_failure() {
    let db = SqlClient::fake();
    let out = db.transaction(Ok(()), || {
        db.insert("t", vec!["x".to_string()]);
        Err::<u32, SqlError>(SqlError::new(SqlReason::Unknown, true, "boom"))
    });
    assert!(out.is_err());
    assert_eq!(db.scan("t"), Vec::<Vec<String>>::new());
}

#[test]
fn savepoints_release_or_hold() {
    let db = SqlClient::fake();
    db.savepoint("sp1", Ok(()), || {
        db.insert("t", vec!["a".to_string()]);
        Ok::<(), SqlError>(())
    })
    .unwrap();
    assert_eq!(db.savepoint_count(), 0);

    let strict = SqlClient::fake().with_savepoints(false);
    strict
        .savepoint("sp1", Ok(()), || Ok::<(), SqlError>(()))
        .unwrap();
    assert_eq!(strict.savepoint_count(), 1);
}

#[test]
fn socket_readers_map_pulls() {
    let (a, b) = socket_pair(4096);
    b.write(b"he".to_vec());
    b.write(b"llo".to_vec());
    drop(b);
    assert_eq!(a.reader_bytes(), Some(b"he".to_vec()));
    assert_eq!(a.reader_string(), Some(Ok("llo".to_string())));
    assert_eq!(a.reader_bytes(), None);
}

#[test]
fn socket_reader_bytes_timeout_expires() {
    let (a, _b) = socket_pair(4096);
    let out = a.reader_bytes_timeout(std::time::Duration::from_millis(50));
    assert_eq!(out, None);
}

#[test]
fn socket_pauses_at_high_water_mark() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (a, b) = socket_pair(1024);
    let written = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|s| {
        s.spawn({
            let written = Arc::clone(&written);
            move || {
                // `b` moves in: it drops after the 8th write, so the
                // channel closes and the drain loop below terminates.
                for _ in 0..8 {
                    b.write(vec![0u8; 1024]);
                    written.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        // The writer blocks once the 1-message buffer fills: it cannot
        // finish all 8 without the reader draining.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(written.load(Ordering::SeqCst) < 8);
        let mut count = 0;
        let start = std::time::Instant::now();
        while a.reader_bytes().is_some() {
            count += 1;
            // Bound the test: a hang fails in seconds, never hours.
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "drain hung"
            );
        }
        assert_eq!(count, 8);
    });
    assert_eq!(written.load(Ordering::SeqCst), 8);
}

#[test]
fn schema_find_all_decodes_or_fails() {
    let db = SqlClient::fake();
    assert_eq!(
        db.query_all("t", |row| row[0].parse::<i64>().map_err(|e| e.to_string())),
        Ok(vec![])
    );
    db.insert("t", vec!["1".to_string()]);
    db.insert("t", vec!["2".to_string()]);
    assert_eq!(
        db.query_all("t", |row| row[0].parse::<i64>().map_err(|e| e.to_string())),
        Ok(vec![1, 2])
    );
    db.insert("t", vec!["nope".to_string()]);
    assert!(db
        .query_all("t", |row| row[0].parse::<i64>().map_err(|e| e.to_string()))
        .is_err());
}

#[test]
fn schema_find_one_and_option() {
    let db = SqlClient::fake();
    let err = db
        .query_one("t", |row| Ok::<String, String>(row[0].clone()))
        .unwrap_err();
    assert_eq!(err.reason, SqlReason::EmptyResult);
    assert_eq!(
        db.query_option("t", |row| Ok::<String, String>(row[0].clone())),
        Ok(None)
    );

    db.insert("t", vec!["a".to_string()]);
    assert_eq!(
        db.query_one("t", |row| Ok::<String, String>(row[0].clone())),
        Ok("a".to_string())
    );
    assert_eq!(
        db.query_option("t", |row| Ok::<String, String>(row[0].clone())),
        Ok(Some("a".to_string()))
    );
    assert!(db
        .query_one("t", |_| Err::<String, String>("bad row".to_string()))
        .is_err());
}

#[test]
fn schema_execute_discards_result() {
    let db = SqlClient::fake();
    db.execute("t", vec!["x".to_string()]);
    assert_eq!(db.scan("t"), vec![vec!["x".to_string()]]);
}

#[test]
fn resolver_skips_batch_when_all_ids_fail_encoding() {
    use sql::BatchResolver;
    let resolver = BatchResolver::new(|_| Err::<String, String>("bad id".to_string()));
    let out = resolver.resolve_each(&["a", "b"], |_| panic!("must not run"));
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|r| r.is_err()));
    assert!(!resolver.executed());
}

#[test]
fn resolver_dedups_and_aligns() {
    use sql::BatchResolver;
    let db = SqlClient::fake();
    db.insert("t", vec!["1".to_string()]);
    let resolver = BatchResolver::new(|id| Ok::<String, String>(id.to_string()));
    let runs = std::sync::atomic::AtomicUsize::new(0);
    let out = resolver.resolve_each(&["1", "1", "2"], |keys| {
        runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        keys.iter()
            .map(|k| db.scan("t").into_iter().find(|r| &r[0] == k).unwrap_or_default())
            .collect()
    });
    assert!(resolver.executed());
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(out[0], out[1]);
    assert_eq!(out[2], Ok(vec![]));
}

#[test]
fn resolver_completes_duplicates_when_encoding_fails() {
    use sql::BatchResolver;
    let resolver = BatchResolver::new(|id| {
        if id == "bad" {
            Err::<String, String>("bad id".to_string())
        } else {
            Ok(id.to_string())
        }
    });
    let out = resolver.resolve_each(&["bad", "bad", "ok"], |keys| {
        assert_eq!(keys, &["ok".to_string()]);
        vec![vec!["row".to_string()]]
    });
    assert!(out[0].is_err());
    assert_eq!(out[0], out[1]);
    assert_eq!(out[2], Ok(vec!["row".to_string()]));
}

#[test]
fn error_guards_match_wrapper_and_reasons() {
    let err = SqlError::new(SqlReason::UniqueViolation { constraint: None }, false, "dup");
    assert_eq!(err.reason_tag(), "UniqueViolation");
    assert!(matches!(err.reason, SqlReason::UniqueViolation { .. }));
    assert!(!matches!(err.reason, SqlReason::ConstraintError { .. }));
    for (reason, tag) in [
        (SqlReason::ConstraintError { detail: None }, "ConstraintError"),
        (SqlReason::ForeignKeyViolation, "ForeignKeyViolation"),
        (SqlReason::SyntaxError, "SyntaxError"),
        (SqlReason::ConnectionLost, "ConnectionLost"),
        (SqlReason::EmptyResult, "EmptyResult"),
        (SqlReason::Unknown, "Unknown"),
    ] {
        let err = SqlError::new(reason, true, "m");
        assert_eq!(err.reason_tag(), tag);
        assert!(err.retryable);
        assert_eq!(err.message, "m");
    }
}

#[test]
fn error_schema_roundtrip_preserves_wrapper() {
    let err = SqlError::new(
        SqlReason::UniqueViolation { constraint: Some("k".to_string()) },
        false,
        "dup key",
    );
    let back = SqlError::from_json(&err.to_json()).unwrap();
    assert_eq!(back, err);
    assert!(SqlError::from_json(&serde_json::json!({"tag": "Nope"})).is_none());
}
