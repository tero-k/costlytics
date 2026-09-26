// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod secrets;

use std::path::PathBuf;
use std::sync::Arc;

use service::CostlyticsService;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let settings_path = match std::env::var_os("COSTLYTICS_CONFIG") {
                Some(p) => PathBuf::from(p),
                None => app.path().app_config_dir()?.join("settings.toml"),
            };
            if let Some(dir) = settings_path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            tracing::info!(path = %settings_path.display(), "loading settings");
            let settings_path_str = settings_path.to_str().ok_or("settings path is not valid UTF-8")?;
            let config = match data::config::AppConfig::load(settings_path_str) {
                Ok(config) => config,
                Err(e) => {
                    tracing::error!(path = %settings_path.display(), error = %e, "failed to load settings");
                    app.dialog()
                        .message(format!(
                            "Costlytics could not load its settings file:\n\n{}\n\n{e}",
                            settings_path.display()
                        ))
                        .kind(MessageDialogKind::Error)
                        .title("Costlytics")
                        .blocking_show();
                    return Err(e.into());
                }
            };
            let svc = Arc::new(CostlyticsService::new(
                config,
                Some(settings_path),
                Arc::new(secrets::KeyringSecretStore),
            ));
            app.manage(Arc::clone(&svc));
            // Register in the background so the window opens immediately;
            // pages wait on `pending` sources (web/src/shared/sourcePicker.ts).
            std::thread::spawn(move || svc.register_all());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::cost_summary,
            commands::cost_timeseries,
            commands::cost_breakdown,
            commands::cost_compare,
            commands::cost_estimate,
            commands::cost_resource_search,
            commands::filter_values_services,
            commands::filter_values_accounts,
            commands::filter_values_regions,
            commands::filter_values_tag_keys,
            commands::filter_values_tag_values,
            commands::sources,
            commands::settings,
            commands::settings_source_save,
            commands::settings_source_delete,
            commands::settings_source_test,
            commands::settings_source_reload,
            commands::settings_cost_guard_save,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Costlytics");
}
