//! Database failures, classified into the handful of cases callers act on differently.

/// What went wrong, at a granularity a caller can branch on.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("the database refused a duplicate: {0}")]
    Duplicate(String),
    #[error("a required row was not found")]
    NotFound,
    #[error("a treasury payment is already in flight")]
    ReleaseInFlight,
    #[error("a constraint was violated: {0}")]
    ConstraintViolated(String),
    #[error("database error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("migrations failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

impl DbError {
    /// Turn a driver error into something a caller can branch on.
    ///
    /// Postgres error codes rather than message matching: `23505` is a unique violation and
    /// `23514` a check violation, in every locale and every version. Matching on message text is
    /// how error handling breaks when somebody upgrades the server.
    pub fn classify(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e {
            let constraint = db.constraint().unwrap_or_default().to_string();
            match db.code().as_deref() {
                Some("23505") => {
                    // F-09: this specific index is the one the API reports as 409, so the caller
                    // can tell "a payment is already in flight" from "you sent that twice".
                    // Two indexes guard the same invariant from different angles: one settlement
                    // in flight per treasury, and one settlement per reserved sequence. Either
                    // firing means the same thing to a caller — this treasury cannot start another
                    // payment right now — so both map to the same answer rather than one of them
                    // arriving as an unexplained 500.
                    if constraint == "settlements_one_in_flight_per_treasury"
                        || constraint == "settlements_one_settlement_per_reserved_sequence"
                    {
                        return DbError::ReleaseInFlight;
                    }
                    return DbError::Duplicate(constraint);
                }
                Some("23514") | Some("23503") => {
                    return DbError::ConstraintViolated(constraint);
                }
                _ => {}
            }
        }
        if matches!(e, sqlx::Error::RowNotFound) {
            return DbError::NotFound;
        }
        DbError::Sql(e)
    }

    pub fn is_duplicate(&self) -> bool {
        matches!(self, DbError::Duplicate(_))
    }

    /// The stable API code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            DbError::ReleaseInFlight => "RELEASE_IN_FLIGHT",
            DbError::Duplicate(_) => "DUPLICATE",
            DbError::NotFound => "NOT_FOUND",
            DbError::ConstraintViolated(_) => "CONSTRAINT_VIOLATED",
            DbError::Sql(_) | DbError::Migrate(_) => "INTERNAL",
        }
    }
}
