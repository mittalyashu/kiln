//! Repo setup and local records (SPEC §3, §5).
//!
//! A repo is a directory holding script files. Creating one initializes git in
//! an empty folder, records it in the local database, and makes it the active
//! script library. The database record is the source of truth for the stable
//! repo id; the folder path is stored canonicalized so later comparisons do not
//! depend on how the user typed it.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::error::AppError;
use crate::git;

const SELECT_REPO: &str = "SELECT id, name, path, remote, is_active FROM repos";
const REMOTE_NAME: &str = "origin";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub remote: Option<String>,
    pub is_active: bool,
}

/// Creates a repository in `folder`, records it locally, and makes it the
/// active script library.
///
/// The folder must not already be a git repository and must be empty. Any
/// existing entry rejects the folder so the user does not accidentally
/// initialize git around unrelated files; an existing work tree should be
/// connected instead.
pub fn create(
    conn: &Connection,
    folder: &Path,
    name: Option<&str>,
    remote: Option<&str>,
) -> Result<Repo, AppError> {
    let remote = normalize_remote(remote);
    let remote_value = remote.as_deref();

    let (folder, created_folder) = prepare_folder(folder)?;
    let name = normalize_name(name, &folder);
    let mut git_touched = false;

    let result = (|| {
        // Reserve the path inside a write transaction before touching the
        // filesystem. A concurrent create for the same folder then fails on
        // the unique path (and rolls back) instead of initializing git twice.
        let tx = conn.unchecked_transaction()?;
        let id = match tx.execute(
            "INSERT INTO repos (path, name, remote, is_active) VALUES (?1, ?2, ?3, 0)",
            params![folder.to_string_lossy(), name, remote_value],
        ) {
            Ok(_) => tx.last_insert_rowid(),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(AppError::InvalidRepository(
                    "This folder is already registered as a script repository.".to_string(),
                ));
            }
            Err(error) => return Err(error.into()),
        };

        ensure_empty(&folder)?;
        // `ensure_empty` just proved there is no `.git` here, so anything git
        // creates from now on is ours to remove if a later step fails.
        git_touched = true;
        git::init(&folder)?;
        if let Some(url) = remote_value {
            git::add_remote(&folder, REMOTE_NAME, url)?;
        }

        tx.execute("UPDATE repos SET is_active = 0 WHERE is_active = 1", [])?;
        tx.execute("UPDATE repos SET is_active = 1 WHERE id = ?1", params![id])?;
        tx.commit()?;

        Ok(Repo {
            id,
            name,
            path: folder.to_string_lossy().into_owned(),
            remote: remote_value.map(str::to_string),
            is_active: true,
        })
    })();

    if result.is_err() && git_touched {
        // We created this git directory, so undo it rather than leaving a
        // half-initialized repository behind. Validation failures before init
        // must never touch an existing work tree.
        cleanup_failed_init(&folder, created_folder);
    } else if result.is_err() && created_folder {
        // The folder itself was created by us and is still empty.
        let _ = fs::remove_dir_all(&folder);
    }

    result
}

/// Lists every known repo. The active repo is included but not reordered.
pub fn list(conn: &Connection) -> Result<Vec<Repo>, AppError> {
    let mut statement = conn.prepare(&format!("{SELECT_REPO} ORDER BY name COLLATE NOCASE, id"))?;
    let rows = statement.query_map([], repo_from_row)?;

    let mut repos = Vec::new();
    for repo in rows {
        repos.push(repo?);
    }
    Ok(repos)
}

/// Returns the repo that was most recently created or selected, if any.
pub fn active(conn: &Connection) -> Result<Option<Repo>, AppError> {
    Ok(conn
        .query_row(
            &format!("{SELECT_REPO} WHERE is_active = 1"),
            [],
            repo_from_row,
        )
        .optional()?)
}

fn prepare_folder(folder: &Path) -> Result<(PathBuf, bool), AppError> {
    if folder.as_os_str().is_empty() {
        return Err(AppError::InvalidRepository(
            "Choose a folder for the script repository.".to_string(),
        ));
    }

    if !folder.is_absolute() {
        return Err(AppError::InvalidRepository(
            "Choose an absolute folder path.".to_string(),
        ));
    }

    let created = if folder.exists() {
        if !folder.is_dir() {
            return Err(AppError::InvalidRepository(
                "The selected path is a file, not a folder.".to_string(),
            ));
        }
        false
    } else {
        fs::create_dir_all(folder)?;
        true
    };

    let canonical = fs::canonicalize(folder)?;
    Ok((canonical, created))
}

fn ensure_empty(folder: &Path) -> Result<(), AppError> {
    if folder.join(".git").exists() {
        return Err(AppError::InvalidRepository(
            "This folder is already a git repository. Connect it instead of creating a new one."
                .to_string(),
        ));
    }

    let mut entries = fs::read_dir(folder)?;
    match entries.next() {
        None => Ok(()),
        Some(Ok(_)) => Err(AppError::InvalidRepository(
            "The folder is not empty. Choose an empty folder to create a new repository, or \
             connect the existing repository instead."
                .to_string(),
        )),
        Some(Err(error)) => Err(AppError::Io(error)),
    }
}

fn normalize_name(name: Option<&str>, folder: &Path) -> String {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| folder_name(folder))
}

fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Scripts")
        .to_string()
}

fn normalize_remote(remote: Option<&str>) -> Option<String> {
    match remote.map(str::trim) {
        None | Some("") => None,
        Some(remote) => Some(remote.to_string()),
    }
}

fn cleanup_failed_init(folder: &Path, created_folder: bool) {
    if created_folder {
        let _ = fs::remove_dir_all(folder);
    } else {
        let _ = fs::remove_dir_all(folder.join(".git"));
    }
}

fn repo_from_row(row: &Row<'_>) -> rusqlite::Result<Repo> {
    let path: String = row.get(2)?;
    let name: Option<String> = row.get(1)?;

    Ok(Repo {
        id: row.get(0)?,
        name: name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| folder_name(Path::new(&path))),
        path,
        remote: row.get(3)?,
        is_active: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("kiln-repo-test-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).expect("temp directory");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory database");
        crate::db::run_migrations(&mut conn).expect("migrations");
        conn
    }

    #[test]
    fn creates_a_git_repo_and_records_it_as_active() {
        let dir = TempDir::new();
        let folder = dir.0.join("scripts");
        let conn = test_connection();

        let repo = create(&conn, &folder, None, None).expect("create repo");

        assert_eq!(repo.name, "scripts");
        assert_eq!(repo.remote, None);
        assert!(repo.is_active);
        assert!(folder.join(".git").is_dir());
        assert_eq!(
            repo.path,
            fs::canonicalize(&folder)
                .expect("canonical")
                .display()
                .to_string()
        );
        assert_eq!(active(&conn).expect("active repo"), Some(repo));
    }

    #[test]
    fn records_an_optional_remote_and_keeps_one_active_repo() {
        let dir = TempDir::new();
        let conn = test_connection();
        let remote = "https://example.com/scripts.git";

        let first = create(&conn, &dir.0.join("first"), None, None).expect("first repo");
        let second = create(
            &conn,
            &dir.0.join("second"),
            Some("Team Scripts"),
            Some(remote),
        )
        .expect("second repo");

        assert_eq!(second.name, "Team Scripts");
        assert_eq!(second.remote.as_deref(), Some(remote));
        let repos = list(&conn).expect("list");
        assert_eq!(repos.len(), 2, "both repos stay registered");
        assert!(
            repos
                .iter()
                .find(|repo| repo.id == second.id)
                .expect("second repo")
                .is_active
        );
        assert!(
            !repos
                .iter()
                .find(|repo| repo.id == first.id)
                .expect("first repo")
                .is_active
        );
        assert_eq!(
            active(&conn).expect("active").map(|repo| repo.id),
            Some(second.id)
        );

        let output = Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(&second.path)
            .output()
            .expect("read remote");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), remote);
    }

    #[test]
    fn rejects_a_non_empty_folder_without_initializing_git() {
        let dir = TempDir::new();
        let folder = dir.0.join("scripts");
        fs::create_dir(&folder).expect("create folder");
        fs::write(folder.join("keep.txt"), "keep").expect("write file");
        let conn = test_connection();

        let error = create(&conn, &folder, None, None).expect_err("non-empty folder");

        assert!(error.to_string().contains("not empty"));
        assert!(!folder.join(".git").exists());
        assert!(list(&conn).expect("list").is_empty());
    }

    #[test]
    fn rejects_an_existing_git_repo_and_points_at_connecting_it() {
        let dir = TempDir::new();
        let folder = dir.0.join("scripts");
        fs::create_dir(&folder).expect("create folder");
        let status = Command::new("git")
            .arg("init")
            .current_dir(&folder)
            .status()
            .expect("git init");
        assert!(status.success());
        let conn = test_connection();

        let error = create(&conn, &folder, None, None).expect_err("existing repo");

        assert!(error.to_string().contains("Connect it instead"));
        assert!(
            folder.join(".git").exists(),
            "a rejected existing repo must not be modified"
        );
        assert!(list(&conn).expect("list").is_empty());
    }

    #[test]
    fn rejects_a_folder_that_is_already_registered() {
        let dir = TempDir::new();
        let folder = dir.0.join("scripts");
        let conn = test_connection();

        create(&conn, &folder, None, None).expect("first create");
        let error = create(&conn, &folder, None, None).expect_err("duplicate create");

        assert!(error.to_string().contains("already registered"));
        assert!(
            folder.join(".git").exists(),
            "a rejected registered repo must not be modified"
        );
        assert_eq!(list(&conn).expect("list").len(), 1);
    }

    #[test]
    fn rejects_a_relative_folder_path() {
        let conn = test_connection();

        let error = create(&conn, Path::new("relative-scripts"), None, None).expect_err("relative");

        assert!(error.to_string().contains("absolute"));
        assert!(list(&conn).expect("list").is_empty());
    }

    #[test]
    fn name_falls_back_to_the_folder_name() {
        let dir = TempDir::new();
        let conn = test_connection();

        let repo =
            create(&conn, &dir.0.join("my-scripts"), Some("   "), None).expect("create repo");

        assert_eq!(repo.name, "my-scripts");
    }
}
