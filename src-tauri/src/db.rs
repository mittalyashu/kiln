//! Local SQLite database (SPEC §5, §6.3).
//!
//! The database lives in the OS app data directory, never in the script repo.
//! Every connection enables WAL mode, a busy timeout, and foreign keys so that
//! a second process (for example a future headless scheduler) can open the
//! same file safely.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

use crate::error::AppError;

pub const DATABASE_FILE_NAME: &str = "kiln.sqlite3";

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

struct Migration {
    version: i64,
    sql: &'static str,
}

/// Versioned, app-owned migrations. Add new entries here; never edit an entry
/// that has already shipped, because existing databases will not re-run it.
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: r#"
        CREATE TABLE repos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            name TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );

        -- Machine-local script state. Local rows are always keyed by repo id
        -- plus the repo-relative path (SPEC §5).
        CREATE TABLE script_local_state (
            repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
            relative_path TEXT NOT NULL,
            working_directory TEXT,
            reviewed_hash TEXT,
            updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            PRIMARY KEY (repo_id, relative_path)
        );

        -- App-wide settings stay local and are never written into the repo.
        CREATE TABLE app_settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );
    "#,
}];

/// Handle to the database file. Connections are opened per operation, which
/// keeps callers from sharing a single connection across Tauri command threads.
#[derive(Clone)]
pub struct Db {
    path: Arc<PathBuf>,
}

impl Db {
    /// Creates the app data directory if needed, opens the database, and
    /// applies any pending migrations.
    pub fn open(data_dir: &Path) -> Result<Self, AppError> {
        fs::create_dir_all(data_dir)?;

        let db = Self {
            path: Arc::new(data_dir.join(DATABASE_FILE_NAME)),
        };
        let mut conn = db.connect()?;
        run_migrations(&mut conn)?;

        Ok(db)
    }

    pub fn connect(&self) -> Result<Connection, AppError> {
        let conn = Connection::open(self.path.as_ref())?;
        configure(&conn)?;
        Ok(conn)
    }

    pub fn path(&self) -> &Path {
        self.path.as_ref().as_path()
    }

    pub fn schema_version(&self) -> Result<i64, AppError> {
        let conn = self.connect()?;
        current_version(&conn)
    }
}

fn configure(conn: &Connection) -> Result<(), AppError> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

fn current_version(conn: &Connection) -> Result<i64, AppError> {
    let has_migrations: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name = 'schema_migrations'
        )",
        [],
        |row| row.get(0),
    )?;

    if !has_migrations {
        return Ok(0);
    }

    Ok(conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?)
}

pub(crate) fn run_migrations(conn: &mut Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );",
    )?;

    for migration in MIGRATIONS {
        // `BEGIN IMMEDIATE` takes the write lock before we check whether this
        // version is already applied, so two processes starting together
        // serialize instead of racing on the same migration.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let applied: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM schema_migrations WHERE version = ?1
            )",
            params![migration.version],
            |row| row.get(0),
        )?;

        if !applied {
            tx.execute_batch(migration.sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at)
                 VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![migration.version],
            )?;
            tx.pragma_update(None, "user_version", migration.version)?;
        }

        tx.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("kiln-db-test-{}-{unique}", std::process::id()));
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn migrations_are_idempotent_and_versioned() {
        let dir = TestDir::new();
        let db = Db::open(&dir.0).expect("first open");

        assert_eq!(db.schema_version().expect("schema version"), 1);

        // Opening the same database again must not re-run the migration.
        drop(db);
        let db = Db::open(&dir.0).expect("second open");
        assert_eq!(db.schema_version().expect("schema version"), 1);
    }

    #[test]
    fn connections_enable_wal_mode() {
        let dir = TestDir::new();
        let db = Db::open(&dir.0).expect("open");

        let conn = db.connect().expect("connect");
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal mode");

        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn repo_scoped_rows_are_keyed_by_repo_and_relative_path() {
        let dir = TestDir::new();
        let db = Db::open(&dir.0).expect("open");
        let conn = db.connect().expect("connect");

        conn.execute("INSERT INTO repos (path) VALUES (?1)", params!["/tmp/one"])
            .expect("insert repo one");
        let repo_one = conn.last_insert_rowid();
        conn.execute("INSERT INTO repos (path) VALUES (?1)", params!["/tmp/two"])
            .expect("insert repo two");
        let repo_two = conn.last_insert_rowid();

        let insert = "INSERT INTO script_local_state (repo_id, relative_path)
                      VALUES (?1, 'scripts/example.sh')";
        conn.execute(insert, params![repo_one])
            .expect("repo one path");
        conn.execute(insert, params![repo_two])
            .expect("repo two path");

        let duplicate = conn.execute(insert, params![repo_one]);
        assert!(duplicate.is_err(), "same repo + path must be unique");
    }

    #[test]
    fn concurrent_writers_on_separate_connections_do_not_lock_fail() {
        let dir = TestDir::new();
        let db = Db::open(&dir.0).expect("open");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

        let mut threads = Vec::new();
        for index in 0..2 {
            let db = db.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                let conn = db.connect().expect("connect");
                barrier.wait();
                conn.execute(
                    "INSERT INTO repos (path) VALUES (?1)",
                    params![format!("/tmp/repo-{index}")],
                )
                .expect("insert repo");
            }));
        }

        for thread in threads {
            thread.join().expect("writer thread");
        }

        let conn = db.connect().expect("connect");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM repos", [], |row| row.get(0))
            .expect("count repos");
        assert_eq!(count, 2);
    }
}
