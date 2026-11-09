//! System `git` integration (SPEC §4.3).
//!
//! Shells out to the user's git binary for status, commit, pull, and push.
//! No libgit2 and no credential management of our own.

use std::path::Path;
use std::process::{Command, Output};

use crate::error::AppError;

/// Initializes a git repository in an existing, empty directory.
pub fn init(path: &Path) -> Result<(), AppError> {
    run(path, &["init"])?;
    Ok(())
}

/// Adds a named remote without fetching from it. Authentication stays with the
/// user's existing SSH agent or credential helper.
pub fn add_remote(path: &Path, name: &str, url: &str) -> Result<(), AppError> {
    run(path, &["remote", "add", name, url])?;
    Ok(())
}

fn run(path: &Path, args: &[&str]) -> Result<Output, AppError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound && path.exists() {
                AppError::Git(
                    "git is not installed or not on PATH. Install git and try again.".to_string(),
                )
            } else {
                AppError::Io(error)
            }
        })?;

    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        return Err(AppError::Git(format!(
            "`git {}` failed with {}.",
            args.join(" "),
            output.status
        )));
    };

    Err(AppError::Git(format!(
        "`git {}` failed: {detail}",
        args.join(" ")
    )))
}
