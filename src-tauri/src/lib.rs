pub mod accounts;
pub mod env;
pub mod error;
pub mod forge;
pub mod git;
pub mod search;
pub mod selftest;
pub mod settings;
pub mod state;
pub mod types;

use std::sync::{Arc, Mutex, RwLock};

use tauri::{Manager, Runtime, Url, Webview};

use crate::accounts::store::KeychainStore;
use crate::env::Env;
use crate::settings::Settings;
use crate::state::AppState;

pub fn run() {
    if let Some(dest) = std::env::args().skip_while(|a| a != "--self-test").nth(1) {
        std::process::exit(if selftest::run(dest.as_ref()) { 0 } else { 1 });
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri::plugin::Builder::<_, ()>::new("navigation")
                .on_navigation(navigation)
                .build(),
        )
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            // Settings written under the app's former identifier move over once.
            let old = config_dir.with_file_name("dev.wdevs.workbench");
            if !config_dir.exists() && old.exists() {
                let _ = std::fs::rename(&old, &config_dir);
            }
            let settings = Settings::load(&config_dir);
            let (env, _) = Env::detect(settings.git_path.as_deref());
            app.manage(AppState {
                config_dir,
                settings: Mutex::new(settings),
                env: RwLock::new(env),
                ops: Default::default(),
                locks: Default::default(),
                tokens: Arc::new(KeychainStore::default()),
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
            settings::set_forge_account,
            settings::set_mr_remote,
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
            git::commands::log,
            git::commands::cancel_log,
            git::commands::commit_details,
            git::commands::branches_containing,
            git::commands::merge_base,
            git::commands::range_log,
            git::commands::hunk_ranges,
            git::commands::fetch_mr,
            accounts::commands::list_accounts,
            accounts::commands::save_account,
            accounts::commands::remove_account,
            accounts::commands::verify_account,
            accounts::commands::repo_accounts,
            forge::commands::forge_hosts,
            forge::commands::commit_url,
            forge::commands::list_mrs,
            forge::commands::current_mr,
            forge::commands::get_mr,
            forge::commands::mr_threads,
            forge::commands::mr_action,
            forge::commands::new_mr,
            forge::commands::create_mr,
            forge::commands::list_pipelines,
            forge::commands::mr_pipelines,
            forge::commands::pipeline_jobs,
            forge::commands::pipeline_action,
            forge::commands::job_action,
            forge::commands::job_log,
            forge::commands::workflows,
            forge::commands::run_pipeline,
            search::commands::quick_open,
            search::commands::find_in_files,
            search::commands::cancel_find,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// The webview stays on the app: links in foreign content open in the browser instead.
fn navigation<R: Runtime>(w: &Webview<R>, url: &Url) -> bool {
    url.scheme() == "tauri"
        || cfg!(debug_assertions)
            && w.config()
                .build
                .dev_url
                .as_ref()
                .is_some_and(|d| d.origin() == url.origin())
}
