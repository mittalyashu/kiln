use tauri::Manager;

mod commands;
mod db;
mod env;
mod error;
mod git;
pub mod injection;
mod notify;
pub mod params;
mod repos;
mod runner;
mod scheduler;
mod scripts;
mod secrets;
mod settings;

pub struct AppState {
    pub db: db::Db,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let database = db::Db::open(&data_dir)?;

            app.manage(AppState { db: database });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::get_settings,
            commands::save_settings,
            commands::get_default_settings,
            commands::create_repository,
            commands::get_active_repository,
            commands::list_repositories,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
