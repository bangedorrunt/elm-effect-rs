//! SQL service (ADR-0002 Phase 12 platform substitutes).
//!
//! [`SqlError`] carries a [`SqlReason`] plus an explicit retryable flag
//! and message. [`classify_sqlite_error`] maps sqlite codes and messages
//! to reasons (explicit unique codes win, check text maps to
//! [`ConstraintError`](SqlReason::ConstraintError), blank input stays
//! [`Unknown`](SqlReason::Unknown)).
//! [`SqlClient`] is a journaled fake: `insert` stages rows, `scan` reads
//! them, `transaction` takes a pre-computed begin result (a failed begin
//! never runs the body and never rolls back), and body failures roll the
//! journal back. Savepoints count while held and release on success
//! unless constructed [`without savepoints`](SqlClient::with_savepoints).

/// Machine-checkable failure reason.
#[derive(Clone, Debug, PartialEq)]
pub enum SqlReason {
    /// Duplicate key or value, with the constraint when known.
    UniqueViolation {
        /// Constraint detail, if any.
        constraint: Option<String>,
    },
    /// Generic constraint failure.
    ConstraintError {
        /// Detail, if any.
        detail: Option<String>,
    },
    /// Referenced row missing.
    ForeignKeyViolation,
    /// Bad SQL text.
    SyntaxError,
    /// Connection lost.
    ConnectionLost,
    /// A query expected rows but got none (`NoSuchElement`).
    EmptyResult,
    /// Anything else.
    Unknown,
}

/// Typed SQL failure.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlError {
    /// Machine reason.
    pub reason: SqlReason,
    /// Retryable flag, set by the raiser.
    pub retryable: bool,
    /// Human message.
    pub message: String,
}

impl SqlError {
    /// Build from reason, retryability, and message.
    pub fn new(reason: SqlReason, retryable: bool, message: &str) -> Self {
        Self { reason, retryable, message: message.to_string() }
    }

    /// True for unique violations.
    pub fn is_unique_violation(&self) -> bool {
        matches!(self.reason, SqlReason::UniqueViolation { .. })
    }
}

impl std::fmt::Display for SqlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SqlError: {}", self.message)
    }
}

impl std::error::Error for SqlError {}

/// Map sqlite codes and messages to reasons.
pub fn classify_sqlite_error(code: Option<&str>, message: &str) -> SqlReason {
    let lower = message.to_lowercase();
    match code {
        Some("SQLITE_CONSTRAINT_UNIQUE") => SqlReason::UniqueViolation {
            constraint: unique_detail(message),
        },
        Some("SQLITE_CONSTRAINT_CHECK") | Some("SQLITE_CONSTRAINT") => {
            SqlReason::ConstraintError { detail: (!message.is_empty()).then(|| message.to_string()) }
        }
        _ => {
            if lower.contains("unique") {
                return SqlReason::UniqueViolation { constraint: unique_detail(message) };
            }
            SqlReason::Unknown
        }
    }
}

fn unique_detail(message: &str) -> Option<String> {
    // `UNIQUE constraint failed: users.email` -> `users.email`.
    message
        .split(':')
        .nth(1)
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != message.trim())
        .map(str::to_string)
        .or_else(|| {
            // `no identifier here`: no dotted detail present.
            if message.contains('.') {
                message.split_whitespace().last().map(str::to_string)
            } else {
                None
            }
        })
}

/// Journaled fake SQL client: no driver, full rollback semantics.
#[derive(Debug, Default)]
pub struct SqlClient {
    tables: std::cell::RefCell<std::collections::BTreeMap<String, Vec<Vec<String>>>>,
    savepoints_held: std::cell::RefCell<usize>,
    strict_savepoints: bool,
}

impl SqlClient {
    /// Empty fake with savepoints enabled.
    pub fn fake() -> Self {
        Self { tables: Default::default(), savepoints_held: std::cell::RefCell::new(0), strict_savepoints: true }
    }

    /// Toggle savepoint support (`with_savepoints(false)` keeps the
    /// count held instead of releasing).
    pub fn with_savepoints(mut self, enabled: bool) -> Self {
        self.strict_savepoints = enabled;
        self
    }

    /// Stage one row.
    pub fn insert(&self, table: &str, row: Vec<String>) {
        self.tables.borrow_mut().entry(table.to_string()).or_default().push(row);
    }

    /// Read staged rows.
    pub fn scan(&self, table: &str) -> Vec<Vec<String>> {
        self.tables.borrow().get(table).cloned().unwrap_or_default()
    }

    /// Held savepoint count (test introspection).
    pub fn savepoint_count(&self) -> usize {
        if self.strict_savepoints {
            0
        } else {
            *self.savepoints_held.borrow()
        }
    }

    /// Run inside a transaction. The begin result is decided up front: a
    /// failed begin never runs the body and never rolls back. Body
    /// failures restore the pre-transaction tables.
    pub fn transaction<R>(
        &self,
        begin: Result<(), SqlError>,
        body: impl FnOnce() -> Result<R, SqlError>,
    ) -> Result<R, SqlError> {
        begin?;
        let snapshot = self.tables.borrow().clone();
        match body() {
            Ok(v) => Ok(v),
            Err(e) => {
                *self.tables.borrow_mut() = snapshot;
                Err(e)
            }
        }
    }

    /// Run inside a savepoint. Success releases (count drops to zero in
    /// strict mode); otherwise the hold stays visible.
    pub fn savepoint<R>(
        &self,
        _name: &str,
        begin: Result<(), SqlError>,
        body: impl FnOnce() -> Result<R, SqlError>,
    ) -> Result<R, SqlError> {
        begin?;
        *self.savepoints_held.borrow_mut() += 1;
        let snapshot = self.tables.borrow().clone();
        let out = body();
        if out.is_ok() && self.strict_savepoints {
            *self.savepoints_held.borrow_mut() = 0;
        } else if out.is_err() {
            *self.tables.borrow_mut() = snapshot;
        }
        out
    }
}

// Result-set decoders

impl SqlClient {
    /// Decode every row (`findAll`): empty tables yield `[]`, and the first
    /// undecodable row fails the whole query.
    pub fn query_all<T>(
        &self,
        table: &str,
        decode: impl Fn(&[String]) -> Result<T, String>,
    ) -> Result<Vec<T>, SqlError> {
        self.scan(table)
            .iter()
            .map(|row| {
                decode(row).map_err(|message| SqlError::new(SqlReason::Unknown, false, &message))
            })
            .collect()
    }

    /// Decode the first row (`findOne`/`head`): no rows is [`EmptyResult`](SqlReason::EmptyResult),
    /// and an undecodable first row fails.
    pub fn query_one<T>(
        &self,
        table: &str,
        decode: impl Fn(&[String]) -> Result<T, String>,
    ) -> Result<T, SqlError> {
        let rows = self.scan(table);
        let first = rows.first().ok_or_else(|| {
            SqlError::new(SqlReason::EmptyResult, false, "no rows returned")
        })?;
        decode(first).map_err(|message| SqlError::new(SqlReason::Unknown, false, &message))
    }

    /// Decode the first row when present (`option`): `None` for no rows.
    pub fn query_option<T>(
        &self,
        table: &str,
        decode: impl Fn(&[String]) -> Result<T, String>,
    ) -> Result<Option<T>, SqlError> {
        match self.query_one(table, decode) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.reason == SqlReason::EmptyResult => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Run a write and discard the result (`execute`).
    pub fn execute(&self, table: &str, row: Vec<String>) {
        self.insert(table, row);
    }
}
