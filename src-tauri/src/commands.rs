//! Tauri IPC layer (SPEC §6.5-6.6).
//!
//! Commands handle actions and return values; events push live state to the
//! frontend. Run logic lives in Rust, never in React.

use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::AppError;
use crate::repos::{self, Repo};
use crate::settings::{self, Settings};
use crate::{AppState, db::Db};

pub const SETTINGS_UPDATED_EVENT: &str = "settings://updated";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub app_version: String,
    pub data_dir: String,
    pub database_path: String,
    pub schema_version: i64,
}

impl AppInfo {
    pub fn collect(app: &AppHandle, db: &Db) -> Result<Self, AppError> {
        let data_dir = app.path().app_data_dir()?;

        Ok(Self {
            app_version: app.package_info().version.to_string(),
            data_dir: data_dir.display().to_string(),
            database_path: db.path().display().to_string(),
            schema_version: db.schema_version()?,
        })
    }
}

#[tauri::command(async)]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> Result<AppInfo, AppError> {
    AppInfo::collect(&app, &state.db)
}

#[tauri::command(async)]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings, AppError> {
    let conn = state.db.connect()?;
    settings::load(&conn)
}

#[tauri::command(async)]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, AppError> {
    let conn = state.db.connect()?;
    settings::save(&conn, &settings)?;

    let saved = settings::load(&conn)?;
    app.emit(SETTINGS_UPDATED_EVENT, &saved)?;
    Ok(saved)
}

#[tauri::command]
pub fn get_default_settings() -> Settings {
    Settings::default()
}

/// Initializes a git repository in an empty folder, records it locally, and
/// makes it the active script library.
#[tauri::command(async)]
pub fn create_repository(
    state: State<'_, AppState>,
    folder: String,
    name: Option<String>,
    remote: Option<String>,
) -> Result<Repo, AppError> {
    let conn = state.db.connect()?;
    repos::create(
        &conn,
        Path::new(&folder),
        name.as_deref(),
        remote.as_deref(),
    )
}

#[tauri::command(async)]
pub fn get_active_repository(state: State<'_, AppState>) -> Result<Option<Repo>, AppError> {
    let conn = state.db.connect()?;
    repos::active(&conn)
}

#[tauri::command(async)]
pub fn list_repositories(state: State<'_, AppState>) -> Result<Vec<Repo>, AppError> {
    let conn = state.db.connect()?;
    repos::list(&conn)
}
