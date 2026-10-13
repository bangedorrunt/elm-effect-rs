//! SQL service + sockets: typed errors, journaled transactions, loopback I/O.

#[path = "../src/socket.rs"]
mod socket;
#[path = "../src/sql.rs"]
mod sql;

use socket::{
    EventKind, Pull, Reader, Socket, SocketError, WebSocketEvent, WebSocketLike, WebSocketListener,
    Writer, socket_pair,
};
use sql::{Faults, SqlClient, SqlError, SqlReason, classify_sqlite_error, is_sql_error, is_sql_error_reason};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
        classify_sqlite_error(Some("SQLITE_CONSTRAINT_UNIQUE"), None, "UNIQUE constraint failed: users.email"),
        SqlReason::UniqueViolation { constraint: Some(name) } if name == "users.email"
    ));
    assert!(matches!(
        classify_sqlite_error(Some("SQLITE_CONSTRAINT_CHECK"), None, "check failed"),
        SqlReason::ConstraintError { .. }
    ));
    assert_eq!(
        classify_sqlite_error(Some("SQLITE_CONSTRAINT_UNIQUE"), None, "no identifier here"),
        SqlReason::UniqueViolation { constraint: None }
    );
    assert_eq!(classify_sqlite_error(None, None, ""), SqlReason::Unknown);
    assert_eq!(
        classify_sqlite_error(Some(""), None, "x"),
        SqlReason::Unknown
    );
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
fn sql_error_wrap_delegates_and_guards() {
    // SqlError.test.ts:59 — the wrapper falls back to the reason tag and
    // takes retryability from the reason; explicit messages win.
    for (reason, retryable) in [
        (SqlReason::ConnectionLost, true),
        (SqlReason::SyntaxError, false),
        (
            SqlReason::UniqueViolation { constraint: None },
            false,
        ),
        (SqlReason::ConstraintError { detail: None }, false),
        (SqlReason::ForeignKeyViolation, false),
        (SqlReason::EmptyResult, false),
        (SqlReason::Unknown, false),
    ] {
        let tag = reason.tag().to_string();
        let wrapped = SqlError::wrap(reason.clone());
        assert_eq!(wrapped.message, tag);
        assert_eq!(wrapped.reason, reason);
        assert_eq!(wrapped.retryable, retryable);
        // SqlError.test.ts:91/103 — guards match their own wrapper type.
        assert!(is_sql_error(&wrapped));
        assert!(is_sql_error_reason(&reason));
    }
    let explicit = SqlError::new(SqlReason::Unknown, true, "custom");
    assert_eq!(explicit.message, "custom");
    assert!(is_sql_error(&explicit));
}

#[test]
fn classify_prefers_explicit_unique_constraint() {
    // SqlError.test.ts:157 — explicit sqlite identifier wins, trimmed.
    assert_eq!(
        classify_sqlite_error(
            Some("SQLITE_CONSTRAINT_UNIQUE"),
            Some(" users_email_key "),
            "UNIQUE constraint failed: users.email"
        ),
        SqlReason::UniqueViolation {
            constraint: Some("users_email_key".to_string())
        }
    );
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
    db.savepoint("sp1", Ok(()), &Faults::default(), || {
        db.insert("t", vec!["a".to_string()]);
        Ok::<(), SqlError>(())
    })
    .unwrap();
    assert_eq!(db.savepoint_count(), 0);

    let strict = SqlClient::fake().with_savepoints(false);
    strict
        .savepoint("sp1", Ok(()), &Faults::default(), || Ok::<(), SqlError>(()))
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
    assert!(
        db.query_all("t", |row| row[0].parse::<i64>().map_err(|e| e.to_string()))
            .is_err()
    );
}

#[test]
fn schema_find_non_empty_or_fails() {
    // SqlSchema.test.ts:48 — non-empty results decode; :60 — no rows is a
    // failure (the pin's NoSuchElementError maps to SqlReason::EmptyResult).
    let db = SqlClient::fake();
    let err = db
        .query_non_empty("t", |row| row[0].parse::<i64>().map_err(|e| e.to_string()))
        .unwrap_err();
    assert_eq!(err.reason, SqlReason::EmptyResult);

    db.insert("t", vec!["1".to_string()]);
    db.insert("t", vec!["2".to_string()]);
    assert_eq!(
        db.query_non_empty("t", |row| row[0].parse::<i64>().map_err(|e| e.to_string())),
        Ok(vec![1, 2])
    );

    // An undecodable row still fails, and its reason is not EmptyResult.
    db.insert("t", vec!["nope".to_string()]);
    let decode_err = db
        .query_non_empty("t", |row| row[0].parse::<i64>().map_err(|e| e.to_string()))
        .unwrap_err();
    assert_ne!(decode_err.reason, SqlReason::EmptyResult);
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
    assert!(
        db.query_option("t", |_| Err::<String, String>("bad row".to_string()))
            .is_err()
    );
    assert!(
        db.query_one("t", |_| Err::<String, String>("bad row".to_string()))
            .is_err()
    );
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
            .map(|k| {
                db.scan("t")
                    .into_iter()
                    .find(|r| &r[0] == k)
                    .unwrap_or_default()
            })
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
    let err = SqlError::new(
        SqlReason::UniqueViolation { constraint: None },
        false,
        "dup",
    );
    assert_eq!(err.reason_tag(), "UniqueViolation");
    assert!(matches!(err.reason, SqlReason::UniqueViolation { .. }));
    assert!(!matches!(err.reason, SqlReason::ConstraintError { .. }));
    for (reason, tag) in [
        (
            SqlReason::ConstraintError { detail: None },
            "ConstraintError",
        ),
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
        SqlReason::UniqueViolation {
            constraint: Some("k".to_string()),
        },
        false,
        "dup key",
    );
    let back = SqlError::from_json(&err.to_json()).unwrap();
    assert_eq!(back, err);
    // SqlError.test.ts:201 — every reason tag roundtrips.
    for reason in [
        SqlReason::UniqueViolation {
            constraint: Some("k".to_string()),
        },
        SqlReason::ConstraintError {
            detail: Some("d".to_string()),
        },
        SqlReason::ForeignKeyViolation,
        SqlReason::SyntaxError,
        SqlReason::ConnectionLost,
        SqlReason::EmptyResult,
        SqlReason::Unknown,
    ] {
        let err = SqlError::new(reason, true, "m");
        assert_eq!(SqlError::from_json(&err.to_json()).unwrap(), err);
    }
    assert!(SqlError::from_json(&serde_json::json!({"tag": "Nope"})).is_none());
}

#[test]
fn failed_savepoint_never_rolls_back() {
    let db = SqlClient::fake();
    db.insert("t", vec!["seed".to_string()]);
    let err = SqlError::new(SqlReason::Unknown, false, "savepoint refused");
    let out = db.savepoint("sp1", Err(err.clone()), &Faults::default(), || {
        Ok::<u32, SqlError>(1)
    });
    assert_eq!(out, Err(err));
    assert_eq!(db.scan("t"), vec![vec!["seed".to_string()]]);
}

#[test]
fn commit_returns_value_on_success() {
    let db = SqlClient::fake();
    let out = db.transaction(Ok(()), || {
        db.insert("t", vec!["x".to_string()]);
        Ok::<u32, SqlError>(7)
    });
    assert_eq!(out, Ok(7));
    assert_eq!(db.scan("t"), vec![vec!["x".to_string()]]);
}

#[test]
fn skipped_release_when_rollback_fails() {
    let db = SqlClient::fake().with_savepoints(false);
    let faults = Faults {
        fail_release: false,
        fail_rollback: true,
    };
    let out = db.savepoint("sp1", Ok(()), &faults, || {
        db.insert("t", vec!["x".to_string()]);
        Err::<u32, SqlError>(SqlError::new(SqlReason::Unknown, true, "boom"))
    });
    assert_eq!(
        out.map_err(|e| e.message),
        Err("rollback failed".to_string())
    );
    assert_eq!(db.savepoint_count(), 1);
}

#[test]
fn outer_rolls_back_when_release_fails() {
    let db = SqlClient::fake();
    let faults = Faults {
        fail_release: true,
        fail_rollback: false,
    };
    let out = db.transaction(Ok(()), || {
        db.savepoint("sp1", Ok(()), &faults, || Ok::<(), SqlError>(()))
            .map(|_| 0u32)
    });
    assert!(out.is_err());
    assert!(db.scan("t").is_empty());
}

// ------------------------------------------ socket abstraction (Socket.ts) ---
//
// `Socket.test.ts` drives a data-shaped web socket: listener bookkeeping,
// pause/resume counters, and dispatched events. The shim below mirrors the
// pin's `TestWebSocket`, so these witnesses need no threads.

struct TestWebSocket {
    listeners: Mutex<HashMap<EventKind, Vec<(WebSocketListener, bool)>>>,
    ready_state: AtomicU8,
    pause_count: AtomicUsize,
    resume_count: AtomicUsize,
}

impl TestWebSocket {
    fn new(ready_state: u8) -> Arc<Self> {
        Arc::new(Self {
            listeners: Mutex::new(HashMap::new()),
            ready_state: AtomicU8::new(ready_state),
            pause_count: AtomicUsize::new(0),
            resume_count: AtomicUsize::new(0),
        })
    }

    fn connecting() -> Arc<Self> {
        Self::new(0)
    }

    fn open() -> Arc<Self> {
        Self::new(1)
    }

    fn dispatch(&self, event: WebSocketEvent) {
        let kind = match event {
            WebSocketEvent::Open => EventKind::Open,
            WebSocketEvent::Message(_) => EventKind::Message,
            WebSocketEvent::Error(_) => EventKind::Error,
            WebSocketEvent::Close { .. } => EventKind::Close,
        };
        // Take the entries out first: `once` listeners are gone either way,
        // and a listener that removes another must not deadlock the dispatch.
        let entries: Vec<(WebSocketListener, bool)> = {
            let mut listeners = self.listeners.lock().unwrap();
            let entries = listeners.remove(&kind).unwrap_or_default();
            let keep: Vec<(WebSocketListener, bool)> = entries
                .iter()
                .filter(|(_, once)| !*once)
                .cloned()
                .collect();
            if !keep.is_empty() {
                listeners.insert(kind, keep);
            }
            entries
        };
        for (listener, _) in entries {
            listener(&event);
        }
    }

    fn listener_count(&self, kind: EventKind) -> usize {
        self.listeners.lock().unwrap().get(&kind).map_or(0, Vec::len)
    }

    fn pause_count(&self) -> usize {
        self.pause_count.load(Ordering::SeqCst)
    }

    fn resume_count(&self) -> usize {
        self.resume_count.load(Ordering::SeqCst)
    }
}

impl WebSocketLike for TestWebSocket {
    fn ready_state(&self) -> u8 {
        self.ready_state.load(Ordering::SeqCst)
    }

    fn pausable(&self) -> bool {
        true
    }

    fn add_event_listener(&self, kind: EventKind, once: bool, listener: WebSocketListener) {
        self.listeners
            .lock()
            .unwrap()
            .entry(kind)
            .or_default()
            .push((listener, once));
    }

    fn remove_event_listener(&self, kind: EventKind, listener: &WebSocketListener) {
        if let Some(entries) = self.listeners.lock().unwrap().get_mut(&kind) {
            entries.retain(|(held, _)| !Arc::ptr_eq(held, listener));
        }
    }

    fn pause(&self) {
        self.pause_count.fetch_add(1, Ordering::SeqCst);
    }

    fn resume(&self) {
        self.resume_count.fetch_add(1, Ordering::SeqCst);
    }

    fn send(&self, _chunk: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn socket_exposes_tls_upgrade_on_the_acquired_reader() {
    // Socket.test.ts:86 — the acquired reader carries the TLS upgrade hook.
    let upgraded = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&upgraded);
    let socket = Socket::make(
        Reader::new(
            || Pull::Empty,
            move || {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        ),
        Writer::new(|_| Ok(())),
    );

    let reader = socket.acquire_reader().expect("a plain socket hands out its reader");
    reader.upgrade().expect("the stub upgrade succeeds");

    assert!(upgraded.load(Ordering::SeqCst));
}

#[test]
fn socket_removes_the_open_listener_when_closing_while_connecting() {
    // Socket.test.ts:118 — a close that arrives before `open` fails the
    // reader acquisition and takes the open listener with it.
    let ws = TestWebSocket::connecting();
    let socket = Socket::from_websocket(ws.clone(), None);
    assert_eq!(
        ws.listener_count(EventKind::Open),
        1,
        "connecting attaches the open listener"
    );

    ws.dispatch(WebSocketEvent::Close {
        code: 1006,
        reason: "closed while connecting".to_string(),
    });

    let failure = socket
        .acquire_reader()
        .expect_err("the reader acquisition fails");
    assert_eq!(
        failure,
        SocketError::Close {
            code: 1006,
            reason: "closed while connecting".to_string()
        }
    );
    assert_eq!(ws.listener_count(EventKind::Open), 0, "and it is removed");
}

#[test]
fn socket_counts_text_frames_by_utf8_length() {
    // Socket.test.ts:155 — one euro sign is three UTF-8 bytes, so a 3-byte
    // mark trips on a single frame.
    let ws = TestWebSocket::open();
    let socket = Socket::from_websocket(ws.clone(), Some(3));

    ws.dispatch(WebSocketEvent::Message("\u{20ac}".as_bytes().to_vec()));

    assert_eq!(ws.pause_count(), 1);
    let reader = socket.acquire_reader().expect("an open socket");
    assert_eq!(reader.pull(), Pull::Chunks(vec!["\u{20ac}".as_bytes().to_vec()]));
}

#[test]
fn socket_uses_the_default_high_water_mark() {
    // Socket.test.ts:168 — a pausable socket with no option uses the 64 KiB
    // default, and draining below it resumes once.
    let ws = TestWebSocket::open();
    let socket = Socket::from_websocket(ws.clone(), None);
    let payload = vec![b'a'; 64 * 1024];

    ws.dispatch(WebSocketEvent::Message(payload.clone()));

    assert_eq!(socket::DEFAULT_HIGH_WATER_MARK, 64 * 1024);
    assert_eq!(ws.pause_count(), 1);
    let reader = socket.acquire_reader().expect("an open socket");
    assert_eq!(reader.pull(), Pull::Chunks(vec![payload]));
    assert_eq!(ws.resume_count(), 1);
}

#[test]
fn socket_reports_a_write_failure_while_a_pull_is_suspended() {
    // Socket.test.ts:184 — `toChannel` writes through the writer, so the
    // failure surfaces without the reader pull ever running.
    let socket = Socket::make(
        Reader::new(
            || -> Pull { panic!("the pull must not run while the write fails") },
            || Err(SocketError::UpgradeUnsupported),
        ),
        Writer::new(|_| Err(SocketError::Write("write failed".to_string()))),
    );

    let exit = socket.to_channel().write_all(&[b"hello".to_vec()]);

    assert_eq!(exit, Err(SocketError::Write("write failed".to_string())));
}
