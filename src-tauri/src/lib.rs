pub mod accounts;
pub mod env;
pub mod error;
pub mod git;
pub mod providers;
pub mod search;
pub mod settings;
pub mod state;
pub mod types;

use std::sync::{Arc, Mutex, RwLock};

use tauri::Manager;

use crate::accounts::store::KeychainStore;
use crate::env::Env;
use crate::settings::Settings;
use crate::state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let settings = Settings::load(&config_dir);
            let env = Env::detect(settings.git_path.as_deref());
            app.manage(AppState {
                config_dir,
                settings: Mutex::new(settings),
                env: RwLock::new(env),
                ops: Default::default(),
                locks: Default::default(),
                tokens: Arc::new(KeychainStore),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::list_repos,
            settings::add_folder,
            settings::remove_folder,
            settings::exclude_repo,
            settings::include_repo,
            settings::set_repo_account,
            settings::set_git_path,
            settings::env_info,
            settings::keymap_overrides,
            settings::open_in_finder,
            git::commands::repo_status,
            git::commands::list_branches,
            git::commands::checkout,
            git::commands::create_branch,
            git::commands::set_upstream,
            git::commands::delete_branch,
            git::commands::delete_remote_branch,
            git::commands::prune,
            git::commands::fetch,
            git::commands::pull,
            git::commands::cancel_op,
            git::commands::list_changes,
            git::commands::compare_base,
            git::commands::file_diff,
            git::commands::list_files,
            git::commands::read_file,
            git::commands::open_in_zed,
            accounts::commands::list_accounts,
            accounts::commands::save_account,
            accounts::commands::remove_account,
            accounts::commands::verify_account,
            accounts::commands::repo_accounts,
            search::commands::quick_open,
            search::commands::find_in_files,
            search::commands::cancel_find,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
