//! App-wide settings (SPEC §5, §9).
//!
//! Settings are persisted in the local database, never inside the script repo.
//! Reads merge the stored values over the defaults so a key that has never been
//! written still resolves to the documented default.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

const SETTINGS_ROW_KEY: &str = "app";

pub const MIN_AUTO_COMMIT_DEBOUNCE_SECONDS: u32 = 5;
pub const MAX_AUTO_COMMIT_DEBOUNCE_SECONDS: u32 = 3600;
pub const MAX_RUN_RETENTION_PER_SCRIPT: u32 = 10_000;
pub const MAX_LOG_SIZE_CAP_MB: u32 = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub auto_commit_enabled: bool,
    pub auto_commit_debounce_seconds: u32,
    pub auto_push: bool,
    pub run_retention_per_script: u32,
    pub log_size_cap_mb: u32,
    pub notifications_enabled: bool,
    pub strict_mode: bool,
    pub path_override: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_commit_enabled: true,
            auto_commit_debounce_seconds: 45,
            auto_push: false,
            run_retention_per_script: 50,
            log_size_cap_mb: 50,
            notifications_enabled: true,
            strict_mode: false,
            path_override: None,
        }
    }
}

impl Settings {
    /// Rejects invalid values with a message the settings UI can display.
    /// Values are never silently clamped.
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_AUTO_COMMIT_DEBOUNCE_SECONDS..=MAX_AUTO_COMMIT_DEBOUNCE_SECONDS)
            .contains(&self.auto_commit_debounce_seconds)
        {
            return Err(format!(
                "Auto-commit debounce must be between {MIN_AUTO_COMMIT_DEBOUNCE_SECONDS} and \
                 {MAX_AUTO_COMMIT_DEBOUNCE_SECONDS} seconds."
            ));
        }

        if self.run_retention_per_script == 0
            || self.run_retention_per_script > MAX_RUN_RETENTION_PER_SCRIPT
        {
            return Err(format!(
                "Run retention must be between 1 and {MAX_RUN_RETENTION_PER_SCRIPT} runs per script."
            ));
        }

        if self.log_size_cap_mb == 0 || self.log_size_cap_mb > MAX_LOG_SIZE_CAP_MB {
            return Err(format!(
                "Log size cap must be between 1 and {MAX_LOG_SIZE_CAP_MB} MB per run."
            ));
        }

        if let Some(path_override) = &self.path_override
            && path_override.trim().is_empty()
        {
            return Err(
                "PATH override must not be empty. Clear the field to remove the override."
                    .to_string(),
            );
        }

        Ok(())
    }
}

pub fn load(conn: &Connection) -> Result<Settings, AppError> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value_json FROM app_settings WHERE key = ?1",
            params![SETTINGS_ROW_KEY],
            |row| row.get(0),
        )
        .optional()?;

    let Some(json) = stored else {
        return Ok(Settings::default());
    };

    // A corrupt blob, or values written by an older build that no longer pass
    // validation, should not brick the settings UI. Fall back to defaults; the
    // next save rewrites the row.
    match serde_json::from_str::<Settings>(&json) {
        Ok(settings) if settings.validate().is_ok() => Ok(settings),
        _ => Ok(Settings::default()),
    }
}

/// Replaces the entire settings object. Callers must send all fields, which the
/// settings UI does with the full typed object.
pub fn save(conn: &Connection, settings: &Settings) -> Result<(), AppError> {
    settings.validate().map_err(AppError::InvalidSettings)?;

    let json = serde_json::to_string(settings)?;
    conn.execute(
        "INSERT INTO app_settings (key, value_json)
         VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET
             value_json = excluded.value_json,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![SETTINGS_ROW_KEY, json],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory database");
        crate::db::run_migrations(&mut conn).expect("migrations");
        conn
    }

    #[test]
    fn defaults_match_the_spec() {
        let settings = Settings::default();

        assert!(settings.auto_commit_enabled);
        assert_eq!(settings.auto_commit_debounce_seconds, 45);
        assert!(!settings.auto_push);
        assert_eq!(settings.run_retention_per_script, 50);
        assert_eq!(settings.log_size_cap_mb, 50);
        assert!(settings.notifications_enabled);
        assert!(!settings.strict_mode);
        assert_eq!(settings.path_override, None);
    }

    #[test]
    fn rejects_out_of_range_values_instead_of_clamping() {
        assert!(
            Settings {
                auto_commit_debounce_seconds: 1,
                ..Default::default()
            }
            .validate()
            .is_err()
        );

        assert!(
            Settings {
                run_retention_per_script: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );

        assert!(
            Settings {
                log_size_cap_mb: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );

        assert!(
            Settings {
                path_override: Some("   ".to_string()),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn accepts_valid_settings() {
        let settings = Settings {
            path_override: Some("/opt/homebrew/bin:/usr/local/bin".to_string()),
            ..Default::default()
        };

        assert!(settings.validate().is_ok());
    }

    #[test]
    fn round_trips_settings_without_losing_defaults() {
        let conn = test_connection();

        assert_eq!(load(&conn).expect("defaults"), Settings::default());

        let settings = Settings {
            auto_push: true,
            strict_mode: true,
            path_override: Some("/usr/local/bin".to_string()),
            ..Default::default()
        };

        save(&conn, &settings).expect("save");
        assert_eq!(load(&conn).expect("load"), settings);
    }

    #[test]
    fn rejects_invalid_settings_without_overwriting_stored_values() {
        let conn = test_connection();
        let settings = Settings {
            run_retention_per_script: 0,
            ..Default::default()
        };

        assert!(save(&conn, &settings).is_err());
        assert_eq!(load(&conn).expect("load"), Settings::default());
    }

    #[test]
    fn corrupt_or_out_of_range_stored_values_fall_back_to_defaults() {
        let conn = test_connection();
        conn.execute(
            "INSERT INTO app_settings (key, value_json) VALUES (?1, ?2)",
            params![SETTINGS_ROW_KEY, "not json"],
        )
        .expect("insert corrupt settings");

        assert_eq!(load(&conn).expect("load"), Settings::default());

        conn.execute(
            "UPDATE app_settings SET value_json = ?1 WHERE key = ?2",
            params![r#"{"runRetentionPerScript":0}"#, SETTINGS_ROW_KEY],
        )
        .expect("insert out-of-range settings");

        assert_eq!(load(&conn).expect("load"), Settings::default());
    }
}
