mod commands;
mod db;
mod drupal;
mod gitlab;
mod models;
mod net;

use commands::*;
use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            use tauri::Manager;
            // macOS gives the system clipboard shortcuts to the first responder
            // through the Edit menu; without a menu there is no responder chain
            // entry for them, so ⌘C/⌘V/⌘A did nothing in the comment box. These
            // are the standard roles - no custom items, no event plumbing.
            {
                use tauri::menu::{MenuBuilder, SubmenuBuilder};
                let app_menu = SubmenuBuilder::new(app, "Drupal Issue Hub")
                    .about(None)
                    .separator()
                    .services()
                    .separator()
                    .hide()
                    .hide_others()
                    .show_all()
                    .separator()
                    .quit()
                    .build()?;
                let edit_menu = SubmenuBuilder::new(app, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;
                let view_menu = SubmenuBuilder::new(app, "View").fullscreen().build()?;
                let window_menu = SubmenuBuilder::new(app, "Window")
                    .minimize()
                    .maximize()
                    .separator()
                    .close_window()
                    .build()?;
                let menu = MenuBuilder::new(app)
                    .items(&[&app_menu, &edit_menu, &view_menu, &window_menu])
                    .build()?;
                app.set_menu(menu)?;
            }
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("issues.db"))?;
            let http = reqwest::Client::builder()
                .user_agent("drupal-issue-hub/0.1 (+https://www.drupal.org)")
                // GitLab's notes API only fills in the note author for
                // authenticated requests; the token comes from the keychain
                // and is attached per-request via auth_headers().
                // Both API hosts sit on the same Fastly edge, which closes
                // idle keep-alive connections aggressively. Keep pooled
                // connections well below that age so a checkout never grabs
                // a connection the server has already dropped.
                .pool_idle_timeout(std::time::Duration::from_secs(15))
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()?;
            app.manage(AppState {
                db: Mutex::new(conn),
                http,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            add_project,
            search_suggestions,
            list_projects,
            list_issues,
            get_issue,
            refresh_project,
            refresh_all,
            remove_project,
            mark_issue_seen,
            mark_project_seen,
            mark_all_seen,
            get_comments,
            get_issue_assets,
            load_more_issues,
            toggle_favorite,
            favorites_count,
            rename_project,
            set_project_notifications,
            reorder_projects,
            get_settings,
            set_settings,
            save_gitlab_token,
            clear_gitlab_token,
            add_issue_comment,
            edit_issue_comment,
            delete_issue_comment,
            set_issue_labels,
            get_project_labels,
            update_issue,
            create_issue,
            set_issue_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
