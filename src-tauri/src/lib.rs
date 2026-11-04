mod commands;

use detached_core::state::AppState;
use tauri::{Manager, Runtime, Url, Webview};

pub fn run() {
    #[cfg(unix)]
    if let Some(dest) = std::env::args().skip_while(|a| a != "--self-test").nth(1) {
        let ok = detached_core::selftest::run(dest.as_ref());
        std::process::exit(if ok { 0 } else { 1 });
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
            app.manage(AppState::boot()?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::list_repos,
            commands::add_folder,
            commands::remove_folder,
            commands::exclude_repo,
            commands::include_repo,
            commands::set_workspace_active,
            commands::save_workspace,
            commands::remove_workspace,
            commands::set_repo_account,
            commands::set_forge_account,
            commands::set_mr_remote,
            commands::set_git_path,
            commands::env_info,
            commands::keymap_overrides,
            commands::open_folder,
            commands::repo_status,
            commands::list_branches,
            commands::checkout,
            commands::create_branch,
            commands::set_upstream,
            commands::delete_branch,
            commands::delete_remote_branch,
            commands::prune,
            commands::fetch,
            commands::pull,
            commands::cancel_op,
            commands::list_changes,
            commands::compare_base,
            commands::file_diff,
            commands::list_files,
            commands::read_file,
            commands::read_image,
            commands::open_in_zed,
            commands::log,
            commands::cancel_log,
            commands::commit_details,
            commands::branches_containing,
            commands::merge_base,
            commands::range_log,
            commands::hunk_ranges,
            commands::fetch_mr,
            commands::list_accounts,
            commands::save_account,
            commands::remove_account,
            commands::verify_account,
            commands::repo_accounts,
            commands::forge_hosts,
            commands::commit_url,
            commands::list_mrs,
            commands::current_mr,
            commands::get_mr,
            commands::mr_threads,
            commands::mr_action,
            commands::new_mr,
            commands::create_mr,
            commands::list_pipelines,
            commands::mr_pipelines,
            commands::pipeline_jobs,
            commands::pipeline_action,
            commands::job_action,
            commands::job_log,
            commands::workflows,
            commands::run_pipeline,
            commands::quick_open,
            commands::find_in_files,
            commands::cancel_find,
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
