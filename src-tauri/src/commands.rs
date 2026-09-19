use crate::db;
use crate::drupal;
use crate::gitlab;
use crate::models::{FetchedComment, FetchedIssue, Project, ProjectSuggestion, RefreshResult, Settings};
use rusqlite::Connection;
use std::sync::Mutex;
use tauri::State;

pub struct AppState {
    pub db: Mutex<Connection>,
    pub http: reqwest::Client,
}

const KEYRING_SERVICE: &str = "drupal-issue-hub";
const KEYRING_ACCOUNT: &str = "gitlab-pat";

/// Map a rusqlite error into the String error type used by all commands.
fn se<T>(r: Result<T, rusqlite::Error>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

fn allowed_host(host: &str) -> bool {
    matches!(host, "drupal.org" | "www.drupal.org" | "git.drupalcode.org")
}

fn get_pat() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("Keychain unavailable: {e}"))?;
    match entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read token from keychain: {e}")),
    }
}

fn project_from_row(row: &db::ProjectRow) -> Project {
    Project {
        id: row.id,
        kind: row.kind.clone(),
        key: row.key.clone(),
        name: row.name.clone(),
        url: row.url.clone(),
        last_refreshed: row.last_refreshed,
        last_error: row.last_error.clone(),
        new_count: row.new_count,
        changed_count: row.changed_count,
        issue_count: row.issue_count,
        notify_new_issues: row.notify_new_issues,
        notify_new_comments: row.notify_new_comments,
    }
}

fn issue_from_row(row: &db::IssueRow) -> crate::models::Issue {
    crate::models::Issue {
        id: row.id,
        project_id: row.project_id,
        project_name: row.project_name.clone(),
        source: row.source.clone(),
        ext_id: row.ext_id.clone(),
        title: row.title.clone(),
        url: row.url.clone(),
        status_label: row.status_label.clone(),
        category_label: row.category_label.clone(),
        version: row.version.clone(),
        author: row.author.clone(),
        created_at: row.created_at,
        changed_at: row.changed_at,
        body: row.body.clone(),
        comment_count: row.comment_count,
        labels: row.labels.clone(),
        favorite: row.favorite,
        is_new: !row.seen,
        has_changes: !row.seen_changed,
    }
}

async fn fetch_for_project(
    http: &reqwest::Client,
    kind: &str,
    key: &str,
    page: usize,
) -> Result<(Vec<FetchedIssue>, bool), String> {
    match kind {
        "drupal" => drupal::fetch_issues(http, key, page).await,
        // The issue list is public: never touch the keychain here. A blocked
        // keychain read (e.g. after the binary changed) would hang every
        // poll. The token is only read for notes/edits, on user action.
        "gitlab" => {
            let (gid, _) = gitlab::resolve_project(http, key, None).await?;
            gitlab::fetch_issues(http, gid, None, page).await
        }
        other => Err(format!("Unknown project kind '{other}'")),
    }
}

/// If a drupal.org project's issue queue is empty but the same machine name
/// on git.drupalcode.org has issues, the project's issues were migrated to
/// GitLab: convert the stored project to a GitLab source and return the
/// updated row. Every d.o project gets a git.drupalcode.org repo, so repo
/// existence alone is no signal — the GitLab side must actually have issues.
async fn maybe_migrate_to_gitlab(
    state: &State<'_, AppState>,
    project: &db::ProjectRow,
) -> Option<db::ProjectRow> {
    if project.kind != "drupal" {
        return None;
    }
    let machine = project
        .url
        .strip_prefix("https://www.drupal.org/project/issues/")?
        .split('/')
        .next()?
        .to_string();
    if machine.is_empty() {
        return None;
    }
    let path = format!("project/{machine}");
    let (gid, _) = gitlab::resolve_project(&state.http, &path, None).await.ok()?;
    // Public read — deliberately no keychain access here (see fetch_for_project).
    let (issues, _) = gitlab::fetch_issues(&state.http, gid, None, 0)
        .await
        .ok()?;
    if issues.is_empty() {
        return None;
    }
    eprintln!(
        "[refresh] {} has no drupal.org issues but {} GitLab work items — migrating source to git.drupalcode.org",
        project.name,
        issues.len()
    );
    let work_items_url = format!("https://git.drupalcode.org/{path}/-/work_items");
    {
        let conn = state.db.lock().unwrap();
        se(
            conn.execute(
                "UPDATE projects SET kind = 'gitlab', key = ?2, url = ?3 WHERE id = ?1",
                rusqlite::params![project.id, path, work_items_url],
            ),
        )
        .ok()?;
    }
    let conn = state.db.lock().unwrap();
    se(db::get_project(&conn, project.id)).ok().flatten()
}

/// Refresh a single project: fetch issues over the network (no DB lock held
/// while waiting) and upsert the result.
async fn refresh_one(state: &State<'_, AppState>, project: &db::ProjectRow) -> RefreshResult {
    let mut project = project.clone();
    // Polls only fetch page 0 (newest issues) — new and changed issues appear
    // at the top; deeper history is lazy-loaded via load_more_issues.
    let mut result = fetch_for_project(&state.http, &project.kind, &project.key, 0).await;

    // Auto-detect projects whose issues moved from drupal.org to GitLab.
    if project.kind == "drupal" {
        if let Ok((issues, _)) = &result {
            if issues.is_empty() {
                if let Some(converted) = maybe_migrate_to_gitlab(state, &project).await {
                    project = converted;
                    result = fetch_for_project(&state.http, &project.kind, &project.key, 0).await;
                }
            }
        }
        // The d.o issue API can also answer 404 once the queue is gone
        // instead of an empty list. Same conversion applies.
        if project.kind == "drupal" {
            if let Err(e) = &result {
                let gone = e.contains("404") || e.contains("Not Found");
                if gone {
                    if let Some(converted) = maybe_migrate_to_gitlab(state, &project).await {
                        project = converted;
                        result = fetch_for_project(&state.http, &project.kind, &project.key, 0).await;
                    }
                }
            }
        }
    }

    match &result {
        Ok((issues, _)) => eprintln!("[refresh] {} ({}): {} issues", project.name, project.kind, issues.len()),
        Err(e) => eprintln!("[refresh] {} ({}): FAILED: {e}", project.name, project.kind),
    }
    let mut conn = state.db.lock().unwrap();
    match result {
        Ok((issues, _has_more)) => {
            match se(db::upsert_issues(&mut conn, project.id, &issues, true)) {
                Ok(delta) => {
                    let _ = db::set_pages_fetched(&conn, project.id, 1);
                    RefreshResult {
                        project_id: project.id,
                        project_name: project.name.clone(),
                        ok: true,
                        error: None,
                        new_issues: delta.new_issues,
                        changed_issues: delta.changed_issues,
                        new_comments: delta.new_comments,
                        target_issue_id: delta.target_issue_id,
                        notify_new_issues: project.notify_new_issues,
                        notify_new_comments: project.notify_new_comments,
                    }
                }
                Err(e) => {
                    let _ = db::record_project_error(&conn, project.id, &e);
                    RefreshResult {
                        project_id: project.id,
                        project_name: project.name.clone(),
                        ok: false,
                        error: Some(e),
                        new_issues: 0,
                        changed_issues: 0,
                        new_comments: 0,
                        target_issue_id: None,
                        notify_new_issues: project.notify_new_issues,
                        notify_new_comments: project.notify_new_comments,
                    }
                }
            }
        }
        Err(e) => {
            let _ = db::record_project_error(&conn, project.id, &e);
            RefreshResult {
                project_id: project.id,
                project_name: project.name.clone(),
                ok: false,
                error: Some(e),
                new_issues: 0,
                changed_issues: 0,
                new_comments: 0,
                target_issue_id: None,
                notify_new_issues: project.notify_new_issues,
                notify_new_comments: project.notify_new_comments,
            }
        }
    }
}

async fn refresh_project_by_id(state: &State<'_, AppState>, project_id: i64) -> RefreshResult {
    let row = {
        let conn = state.db.lock().unwrap();
        se(db::get_project(&conn, project_id)).ok().flatten()
    };
    match row {
        Some(row) => refresh_one(state, &row).await,
        None => RefreshResult {
            project_id,
            project_name: String::new(),
            ok: false,
            error: Some("Project not found".to_string()),
            new_issues: 0,
            changed_issues: 0,
            new_comments: 0,
            target_issue_id: None,
            notify_new_issues: false,
            notify_new_comments: false,
        },
    }
}

#[tauri::command]
pub async fn add_project(state: State<'_, AppState>, url: String) -> Result<Project, String> {
    let input = url.trim().to_string();
    let normalized = if input.contains("://") {
        input.clone()
    } else {
        format!("https://{input}")
    };
    let parsed =
        reqwest::Url::parse(&normalized).map_err(|e| format!("'{input}' is not a valid URL: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("Only https:// URLs are supported".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_string();
    if !allowed_host(&host) {
        return Err(format!(
            "Unsupported host '{host}'. Use www.drupal.org or git.drupalcode.org URLs."
        ));
    }
    let segments: Vec<String> = parsed
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default();

    let http = &state.http;
    let machine_name_of = |segments: &[String]| -> Option<String> {
        segments
            .get(1)
            .filter(|_| segments.first().map(|s| s.as_str()) == Some("project"))
            .cloned()
            .or_else(|| {
                if segments.len() == 1 {
                    Some(segments[0].clone())
                } else {
                    None
                }
            })
    };
    let (kind, key, name, canonical_url) = if host.ends_with("drupal.org") {
        let machine_name = machine_name_of(&segments).ok_or_else(|| {
            "Could not read a project machine name from the URL. Expected https://www.drupal.org/project/<name>".to_string()
        })?;
        let path = format!("project/{machine_name}");
        // Decide by where the work items actually are: GitLab first. The
        // drupal.org issue API still answers for projects whose queue has
        // moved (menu_block_title: 25 rows on d.o, 3 on GitLab), so the
        // d.o lookup alone is no longer a reliable source-of-truth - and
        // only a GitLab source can be commented on and edited in this app.
        let gitlab_live = match gitlab::resolve_project(http, &path, None).await {
            Ok((gid, _)) => {
                let issues = gitlab::fetch_issues(http, gid, None, 0).await.ok();
                let count = issues.as_ref().map(|(l, _)| l.len()).unwrap_or(0);
                eprintln!(
                    "[add] {machine_name}: git.drupalcode.org/{path} has {count} work items"
                );
                count > 0
            }
            Err(e) => {
                eprintln!("[add] GitLab lookup failed for {path}: {e}");
                false
            }
        };
        if gitlab_live {
            let (_, gname) = gitlab::resolve_project(http, &path, None).await
                .map_err(|e| format!("GitLab resolve failed for {path}: {e}"))?;
            return add_gitlab_project(&state, &path, gname).await;
        }
        let (nid, title) = match drupal::resolve_project(http, &machine_name).await {
            Ok(v) => v,
            // The machine name exists on drupal.org but its issue queue has
            // moved to GitLab (markdownify is the example: the project page
            // still loads, but the node.json endpoint is a 302 to a 404 on
            // new.drupal.org). The GitLab branch above covers the "live
            // issues" case; this covers "d.o endpoint is dead" without the
            // user having to know the difference.
            Err(drupal_err) => {
                eprintln!(
                    "[add] d.o lookup failed ({drupal_err}); trying git.drupalcode.org/{path}"
                );
                match gitlab::resolve_project(http, &path, None).await {
                    Ok((_, gname)) => {
                        return add_gitlab_project(&state, &path, gname).await;
                    }
                    Err(gitlab_err) => {
                        return Err(format!(
                            "Project '{machine_name}' was not found on drupal.org ({drupal_err}) \
                             or on git.drupalcode.org ({gitlab_err}). \
                             Add it on git.drupalcode.org directly: https://git.drupalcode.org/{path}"
                        ))
                    }
                }
            }
        };
        (
            "drupal",
            nid,
            title,
            format!("https://www.drupal.org/project/issues/{machine_name}"),
        )
    } else {
        let path = segments
            .first()
            .zip(segments.get(1))
            .map(|(a, b)| format!("{a}/{b}"))
            .ok_or_else(|| {
                "Could not read a project path from the URL. Expected https://git.drupalcode.org/project/<name>".to_string()
            })?;
        let (_, name) = gitlab::resolve_project(http, &path, None).await?;
        let work_items_url = format!("https://git.drupalcode.org/{path}/-/work_items");
        ("gitlab", path, name, work_items_url)
    };
    let (kind, key) = (kind.to_string(), key);

    let existing = {
        let conn = state.db.lock().unwrap();
        se(db::find_project_by_key(&conn, &kind, &key))?
    };
    let project_id = match existing {
        Some(id) => id,
        None => {
            let conn = state.db.lock().unwrap();
            se(conn.execute(
                "INSERT INTO projects (kind, key, name, url, sort_order)
                 VALUES (?1, ?2, ?3, ?4, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM projects))",
                rusqlite::params![kind, key, name, canonical_url],
            ))?;
            conn.last_insert_rowid()
        }
    };

    // Initial fetch so the project shows up with content right away.
    let result = refresh_project_by_id(&state, project_id).await;
    let conn = state.db.lock().unwrap();
    let row = se(db::get_project(&conn, project_id))?.ok_or("Project vanished while adding")?;
    if let Some(err) = result.error {
        // Soft failure: the project is added, but the first fetch failed.
        return Ok(Project {
            last_error: Some(err),
            ..project_from_row(&row)
        });
    }
    Ok(project_from_row(&row))
}

/// Insert a GitLab project by its path, do the initial fetch, and return
/// the row - the same tail that add_project runs for native GitLab URLs,
/// reused by the drupal.org fallback when a d.o lookup comes back empty and
/// the same machine name has live work items on GitLab.
async fn add_gitlab_project(
    state: &State<'_, AppState>,
    path: &str,
    name: String,
) -> Result<Project, String> {
    let work_items_url = format!("https://git.drupalcode.org/{path}/-/work_items");
    let existing = {
        let conn = state.db.lock().unwrap();
        se(db::find_project_by_key(&conn, "gitlab", &path.to_string()))?
    };
    let project_id = match existing {
        Some(id) => id,
        None => {
            let conn = state.db.lock().unwrap();
            se(conn.execute(
                "INSERT INTO projects (kind, key, name, url, sort_order)
                 VALUES (?1, ?2, ?3, ?4, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM projects))",
                rusqlite::params!["gitlab", path, name, work_items_url],
            ))?;
            conn.last_insert_rowid()
        }
    };

    // Initial fetch so the project shows up with content right away.
    let result = refresh_project_by_id(state, project_id).await;
    let conn = state.db.lock().unwrap();
    let row = se(db::get_project(&conn, project_id))?.ok_or("Project vanished while adding")?;
    if let Some(err) = result.error {
        // Soft failure: the project is added, but the first fetch failed.
        return Ok(Project {
            last_error: Some(err),
            ..project_from_row(&row)
        });
    }
    Ok(project_from_row(&row))
}

#[tauri::command]
pub async fn search_suggestions(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<ProjectSuggestion>, String> {
    let q = query.trim().to_string();
    if q.len() < 2 {
        return Ok(vec![]);
    }
    let mut out = drupal::search_projects(&state.http, &q).await?;
    // The drupal.org API still answers for projects whose queue moved to
    // GitLab, so the d.o search would hand back a drupal.org link and the
    // project would be stored read-only. Check the GitLab side for each
    // machine name and prefer it when it actually has work items.
    for p in out.iter_mut() {
        let Some(machine) = p
            .add_url
            .strip_prefix("https://www.drupal.org/project/")
            .map(str::to_string)
        else {
            continue;
        };
        let path = format!("project/{machine}");
        if let Ok((gid, _)) = gitlab::resolve_project(&state.http, &path, None).await {
            let issues = gitlab::fetch_issues(&state.http, gid, None, 0).await.ok();
            if issues.map(|(l, _)| l.len()).unwrap_or(0) > 0 {
                p.kind = "gitlab".into();
                p.id = format!("{gid}");
                p.add_url = format!("https://git.drupalcode.org/{path}");
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let conn = state.db.lock().unwrap();
    Ok(se(db::list_projects(&conn))?
        .iter()
        .map(project_from_row)
        .collect())
}

#[tauri::command]
pub fn list_issues(
    state: State<'_, AppState>,
    project_id: i64,
    unseen_only: bool,
    favorites_only: bool,
) -> Result<Vec<crate::models::Issue>, String> {
    let conn = state.db.lock().unwrap();
    let rows = se(db::list_issues(&conn, project_id, unseen_only, favorites_only))?;
    Ok(rows.iter().map(issue_from_row).collect())
}

/// One issue by primary key. Following a notification has to resolve an id
/// that was current when the banner fired, and `list_issues` is filtered by
/// project and by the unseen/favorites switches, so it cannot stand in here.
#[tauri::command]
pub fn get_issue(
    state: State<'_, AppState>,
    issue_id: i64,
) -> Result<Option<crate::models::Issue>, String> {
    let conn = state.db.lock().unwrap();
    Ok(se(db::get_issue(&conn, issue_id))?
        .as_ref()
        .map(issue_from_row))
}

#[tauri::command]
pub fn toggle_favorite(state: State<'_, AppState>, issue_id: i64) -> Result<bool, String> {
    let conn = state.db.lock().unwrap();
    se(db::toggle_favorite(&conn, issue_id))
}

#[tauri::command]
pub fn favorites_count(state: State<'_, AppState>) -> Result<i64, String> {
    let conn = state.db.lock().unwrap();
    se(db::favorites_count(&conn))
}

#[tauri::command]
pub fn rename_project(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Project name must not be empty".into());
    }
    let conn = state.db.lock().unwrap();
    se(db::rename_project(&conn, project_id, &name))
}

/// Per-module notification switches: new issues and new comments each on/off.
#[tauri::command]
pub fn set_project_notifications(
    state: State<'_, AppState>,
    project_id: i64,
    notify_new_issues: bool,
    notify_new_comments: bool,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    se(db::set_notifications(
        &conn,
        project_id,
        notify_new_issues,
        notify_new_comments,
    ))
}

#[tauri::command]
pub fn reorder_projects(state: State<'_, AppState>, project_ids: Vec<i64>) -> Result<(), String> {
    let mut conn = state.db.lock().unwrap();
    se(db::reorder_projects(&mut conn, &project_ids))
}

#[tauri::command]
pub async fn refresh_project(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<RefreshResult, String> {
    Ok(refresh_project_by_id(&state, project_id).await)
}

#[tauri::command]
pub async fn refresh_all(state: State<'_, AppState>) -> Result<Vec<RefreshResult>, String> {
    let rows = {
        let conn = state.db.lock().unwrap();
        se(db::list_projects(&conn))?
    };
    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        results.push(refresh_one(&state, row).await);
    }
    Ok(results)
}

#[tauri::command]
pub fn remove_project(state: State<'_, AppState>, project_id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    se(db::delete_project(&conn, project_id))
}

#[tauri::command]
pub fn mark_issue_seen(state: State<'_, AppState>, issue_id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    se(db::mark_issue_seen(&conn, issue_id))
}

#[tauri::command]
pub fn mark_project_seen(state: State<'_, AppState>, project_id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    se(db::mark_project_seen(&conn, project_id))
}

#[tauri::command]
pub fn mark_all_seen(state: State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    se(db::mark_all_seen(&conn))
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let conn = state.db.lock().unwrap();
    let poll_enabled = se(db::get_setting(&conn, "poll_enabled"))?
        .map(|v| v != "0")
        .unwrap_or(true);
    let poll_interval_minutes = se(db::get_setting(&conn, "poll_interval"))?
        .and_then(|v| v.parse().ok())
        .unwrap_or(15u64);
    let hide_system_comments = se(db::get_setting(&conn, "hide_system_comments"))?
        .map(|v| v == "1")
        .unwrap_or(false);
    let gitlab_user = se(db::get_setting(&conn, "gitlab_user"))?;
    // The table filter a freshly opened project starts in. Plain key/value row,
    // no migration; "attention" is the app's default when the row is absent.
    let default_mode = se(db::get_setting(&conn, "default_mode"))?
        .filter(|v| v == "attention" || v == "all")
        .unwrap_or_else(|| "attention".to_string());
    // Colour scheme: "dark", "light" or "system". Plain key/value row, no
    // migration; "system" (follow the OS) is the default when the row is absent.
    let theme = se(db::get_setting(&conn, "theme"))?
        .filter(|v| v == "dark" || v == "light" || v == "system")
        .unwrap_or_else(|| "system".to_string());
    Ok(Settings {
        poll_enabled,
        poll_interval_minutes,
        hide_system_comments,
        default_mode,
        theme,
        gitlab_user,
    })
}

#[tauri::command]
pub fn set_settings(
    state: State<'_, AppState>,
    poll_enabled: bool,
    poll_interval_minutes: u64,
    hide_system_comments: bool,
    default_mode: Option<String>,
    theme: Option<String>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    se(db::set_setting(
        &conn,
        "poll_enabled",
        if poll_enabled { "1" } else { "0" },
    ))?;
    se(db::set_setting(
        &conn,
        "poll_interval",
        &poll_interval_minutes.to_string(),
    ))?;
    // Plain key/value row, so this needs no schema migration.
    se(db::set_setting(
        &conn,
        "hide_system_comments",
        if hide_system_comments { "1" } else { "0" },
    ))?;
    // `None` leaves the stored value alone - the modal may save only some of the
    // settings, and only "attention"/"all" are ever written.
    if let Some(mode) = default_mode.filter(|m| m == "attention" || m == "all") {
        se(db::set_setting(&conn, "default_mode", &mode))?;
    }
    // Same opt-out contract: only a valid "dark"/"light"/"system" is ever
    // written, and None (or an invalid value) leaves the stored theme alone.
    if let Some(t) = theme.filter(|t| t == "dark" || t == "light" || t == "system") {
        se(db::set_setting(&conn, "theme", &t))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn save_gitlab_token(
    state: State<'_, AppState>,
    token: String,
) -> Result<String, String> {
    let username = gitlab::validate_token(&state.http, &token).await?;
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("Keychain unavailable: {e}"))?;
    entry
        .set_password(&token)
        .map_err(|e| format!("Could not store token in keychain: {e}"))?;
    let conn = state.db.lock().unwrap();
    se(db::set_setting(&conn, "gitlab_user", &username))?;
    Ok(username)
}

#[tauri::command]
pub fn clear_gitlab_token(state: State<'_, AppState>) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("Keychain unavailable: {e}"))?;
    match entry.delete_credential() {
        Ok(_) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(format!("Could not remove token: {e}")),
    }
    let conn = state.db.lock().unwrap();
    se(conn
        .execute("DELETE FROM settings WHERE key = 'gitlab_user'", [])
        .map(|_| ()))
}

/// Load everything needed to write to a GitLab issue: numeric project id,
/// issue iid and the token. Fails for drupal.org issues (read-only API).
/// Load the discussion thread of an issue. Comments are fetched on demand
/// (one request storm per issue would be wasteful on every poll), cached in
/// SQLite, and new ones flagged. For drupal.org only comments not already
/// cached are downloaded, capped at the newest 60 per open; GitLab notes are
/// fetched in one request and require a token.
#[tauri::command]
pub async fn get_comments(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    issue_id: i64,
) -> Result<Vec<crate::models::Comment>, String> {
    let (source, project_key, project_id, ext_id) = {
        let conn = state.db.lock().unwrap();
        let issue = se(db::get_issue(&conn, issue_id))?.ok_or("Issue not found")?;
        let project = se(db::get_project(&conn, issue.project_id))?.ok_or("Project not found")?;
        (issue.source, project.key, issue.project_id, issue.ext_id)
    };

    // `complete` marks a thread that was read to its end, which is what makes
    // pruning cached notes safe below. The drupal.org path deliberately fetches
    // only a subset, so it never is.
    let (fetched, complete): (Vec<FetchedComment>, bool) = match source.as_str() {
        "drupal" => {
            // Node fetch doubles as an edit checkup for the issue itself:
            // if it changed since the last fetch, title/body/status are
            // updated and the issue gets its UPD flag.
            let (issue_fetched, ids) = drupal::fetch_issue_node(&state.http, &ext_id).await?;
            {
                let mut conn = state.db.lock().unwrap();
                se(db::upsert_issues(&mut conn, project_id, std::slice::from_ref(&issue_fetched), true))?;
            }
            let cached = {
                let conn = state.db.lock().unwrap();
                se(db::cached_comment_ids(&conn, issue_id))?
            };
            // Uncached comments (new ones, newest first), capped so opening an
            // ancient issue doesn't fire 300 requests…
            let missing: Vec<&String> = ids
                .iter()
                .filter(|id| !cached.contains(id))
                .take(60)
                .collect();
            // …plus a re-fetch of the newest already-cached comments, so
            // edits to recent comments are picked up while viewing.
            let recent: Vec<&String> = ids
                .iter()
                .filter(|id| cached.contains(id))
                .take(10)
                .collect();
            let wanted: Vec<String> = missing
                .into_iter()
                .chain(recent)
                .map(|id| id.clone())
                .collect();
            let total = wanted.len();
            use tauri::Emitter;
            let mut done = 0usize;
            let mut out = Vec::with_capacity(wanted.len());
            for chunk in wanted.chunks(8) {
                let mut handles = Vec::with_capacity(chunk.len());
                for cid in chunk {
                    let http = state.http.clone();
                    let cid = cid.clone();
                    handles.push(tauri::async_runtime::spawn(async move {
                        drupal::fetch_comment(&http, &cid).await
                    }));
                }
                for handle in handles {
                    if let Some(c) = handle
                        .await
                        .map_err(|e| format!("comment fetch task failed: {e}"))??
                    {
                        out.push(c);
                    }
                }
                done += chunk.len();
                let _ = app.emit(
                    "comments-progress",
                    serde_json::json!({
                        "issueId": issue_id,
                        "done": done.min(total),
                        "total": total
                    }),
                );
            }
            (out, false)
        }
        "gitlab" => {
            // The whole thread is re-read on every open (paged), so edited
            // comments are refreshed too and deleted ones can be pruned.
            //
            // git.drupalcode.org serves the notes list only to authenticated
            // requests (401 without a token). The API's `author` field is
            // empty for notes written by *other* users even with a valid
            // token, so the stored author name is preserved on upsert -
            // it is only filled from the API when the API actually sends
            // one (your own notes, or notes the instance populates).
            let token = get_pat()?.ok_or_else(|| {
                "Reading comments on git.drupalcode.org requires a personal access token (Settings).".to_string()
            })?;
            let (gid, _) = gitlab::resolve_project(&state.http, &project_key, Some(&token)).await?;
            let (fetched, complete) =
                gitlab::fetch_comments(&state.http, gid, &ext_id, Some(token.as_str())).await?;
            (fetched, complete)
        }
        other => return Err(format!("Unknown source '{other}'")),
    };

    let comments = {
        let mut conn = state.db.lock().unwrap();
        // GitLab hands back the thread in pages of 100; when the last page was
        // short we have read all of it, so notes that vanished upstream (here
        // or in the web UI) can be dropped from the cache. A capped or partial
        // read, and the drupal.org path, must never prune.
        if complete {
            let keep: Vec<String> = fetched.iter().map(|c| c.ext_id.clone()).collect();
            se(db::prune_comments(&conn, issue_id, &keep))?;
        }
        // GitLab omits the author on notes it does not attribute to the
        // caller, so the upsert keeps the stored name whenever the fetch
        // came back with an empty one.
        let preserve = source == "gitlab";
        let rows = se(db::upsert_comments(&mut conn, issue_id, &fetched, preserve))?;
        se(db::mark_comments_seen(&conn, issue_id))?;
        rows
    };
    Ok(comments)
}

/// Lazy-load the next page of an older issue backlog into the cache. New
/// inserts are marked seen: they are history, not fresh activity.
#[tauri::command]
pub async fn load_more_issues(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<crate::models::LoadMoreResult, String> {
    let (row, pages_fetched) = {
        let conn = state.db.lock().unwrap();
        let row = se(db::get_project(&conn, project_id))?.ok_or("Project not found")?;
        let pages = se(db::get_pages_fetched(&conn, project_id))?;
        (row, pages)
    };
    let (issues, has_more) =
        fetch_for_project(&state.http, &row.kind, &row.key, pages_fetched as usize).await?;
    let loaded = issues.len() as i64;
    {
        let mut conn = state.db.lock().unwrap();
        se(db::upsert_issues(&mut conn, row.id, &issues, false))?;
        se(db::set_pages_fetched(&conn, row.id, pages_fetched + 1))?;
    }
    Ok(crate::models::LoadMoreResult { loaded, has_more })
}

/// Patches and merge requests belonging to an issue. For drupal.org the
/// rendered issue page is scraped (the file API is closed); for GitLab the
/// issue description and cached comments are scanned for MR references.
#[tauri::command]
pub async fn get_issue_assets(
    state: State<'_, AppState>,
    issue_id: i64,
) -> Result<Vec<crate::models::AssetLink>, String> {
    let (source, issue_url, body) = {
        let conn = state.db.lock().unwrap();
        let issue = se(db::get_issue(&conn, issue_id))?.ok_or("Issue not found")?;
        (issue.source, issue.url, issue.body)
    };
    match source.as_str() {
        "drupal" => drupal::fetch_issue_assets(&state.http, &issue_url).await,
        "gitlab" => {
            let comment_bodies = {
                let conn = state.db.lock().unwrap();
                se(db::comment_bodies(&conn, issue_id))?
            };
            // A bare !183 is built against this issue's own project path, but
            // it often means *another* project's MR — verify the constructed
            // links so the panel only offers references that actually resolve.
            let mrs = gitlab::extract_mrs(&body, &comment_bodies, &issue_url);
            Ok(gitlab::prune_missing_mrs(&state.http, mrs).await)
        }
        other => Err(format!("Unknown source '{other}'")),
    }
}

async fn gitlab_context_for_issue(
    state: &State<'_, AppState>,
    issue_id: i64,
) -> Result<(i64, String, String), String> {
    let (project_key, ext_id) = {
        let conn = state.db.lock().unwrap();
        let issue = se(db::get_issue(&conn, issue_id))?.ok_or("Issue not found")?;
        if issue.source != "gitlab" {
            return Err(
                "This issue lives in the drupal.org queue, which does not offer a write API."
                    .to_string(),
            );
        }
        let project = se(db::get_project(&conn, issue.project_id))?.ok_or("Project not found")?;
        (project.key, issue.ext_id)
    };
    let token = get_pat()?.ok_or_else(|| {
        "No GitLab token configured. Add a personal access token in Settings to edit issues."
            .to_string()
    })?;
    let (gid, _) = gitlab::resolve_project(&state.http, &project_key, Some(&token)).await?;
    Ok((gid, ext_id, token))
}

/// The project an issue row belongs to, for the refresh that follows a write.
fn project_id_for_issue(state: &State<'_, AppState>, issue_id: i64) -> Result<i64, String> {
    let conn = state.db.lock().unwrap();
    Ok(se(db::get_issue(&conn, issue_id))?.ok_or("Issue not found")?.project_id)
}

/// Same as the issue variant, but straight from a project row: creating an
/// issue does not have a row to look back from.
async fn gitlab_context_for_project(
    state: &State<'_, AppState>,
    project_id: i64,
) -> Result<(i64, String), String> {
    let project_key = {
        let conn = state.db.lock().unwrap();
        let project = se(db::get_project(&conn, project_id))?.ok_or("Project not found")?;
        if project.kind != "gitlab" {
            return Err(
                "This project lives in the drupal.org queue, which does not offer a write API."
                    .to_string(),
            );
        }
        project.key
    };
    let token = get_pat()?.ok_or_else(|| {
        "No GitLab token configured. Add a personal access token in Settings to create issues."
            .to_string()
    })?;
    let (gid, _) = gitlab::resolve_project(&state.http, &project_key, Some(&token)).await?;
    Ok((gid, token))
}

async fn refresh_after_write(state: &State<'_, AppState>, project_id: i64) {
    if let Ok(Some(row)) = {
        let conn = state.db.lock().unwrap();
        se(db::get_project(&conn, project_id))
    } {
        refresh_one(state, &row).await;
    }
}

#[tauri::command]
pub async fn add_issue_comment(
    state: State<'_, AppState>,
    issue_id: i64,
    body: String,
) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("Comment is empty".into());
    }
    let (gid, iid, token) = gitlab_context_for_issue(&state, issue_id).await?;
    gitlab::add_comment(&state.http, gid, &iid, &token, &body).await?;
    refresh_after_write(&state, project_id_for_issue(&state, issue_id)?).await;
    Ok(())
}

/// Rewrite one of your own comments on a GitLab issue. `ext_id` is the note id
/// as stored in the cache; GitLab rejects edits of other people's notes.
#[tauri::command]
pub async fn edit_issue_comment(
    state: State<'_, AppState>,
    issue_id: i64,
    ext_id: String,
    body: String,
) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("Comment is empty".into());
    }
    let (gid, iid, token) = gitlab_context_for_issue(&state, issue_id).await?;
    gitlab::edit_comment(&state.http, gid, &iid, &ext_id, &token, &body).await?;
    refresh_after_write(&state, project_id_for_issue(&state, issue_id)?).await;
    Ok(())
}

/// Delete one of your own comments on a GitLab issue, for everyone. The cached
/// row goes with it: `get_comments` would also drop it on the next open, but
/// only when the note list fits in one page.
#[tauri::command]
pub async fn delete_issue_comment(
    state: State<'_, AppState>,
    issue_id: i64,
    ext_id: String,
) -> Result<(), String> {
    let (gid, iid, token) = gitlab_context_for_issue(&state, issue_id).await?;
    gitlab::delete_comment(&state.http, gid, &iid, &ext_id, &token).await?;
    {
        let conn = state.db.lock().unwrap();
        se(db::delete_comment(&conn, issue_id, &ext_id))?;
    }
    refresh_after_write(&state, project_id_for_issue(&state, issue_id)?).await;
    Ok(())
}

#[tauri::command]
pub async fn update_issue(
    state: State<'_, AppState>,
    issue_id: i64,
    title: String,
    description: String,
) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err("Title must not be empty".into());
    }
    let (gid, iid, token) = gitlab_context_for_issue(&state, issue_id).await?;
    gitlab::update_issue(
        &state.http,
        gid,
        &iid,
        &token,
        Some(&title),
        Some(&description),
        None,
    )
    .await?;
    refresh_after_write(&state, project_id_for_issue(&state, issue_id)?).await;
    Ok(())
}

/// Create a new issue on a git.drupalcode.org project. Returns the new work
/// item's iid so the UI can open it. The description is written verbatim -
/// GitLab renders it as markdown, which is what the existing edit form
/// handles too. The local cache refreshes afterwards, so the new issue shows
/// up in the list without waiting for the poll.
#[tauri::command]
pub async fn create_issue(
    state: State<'_, AppState>,
    project_id: i64,
    title: String,
    description: String,
) -> Result<String, String> {
    if title.trim().is_empty() {
        return Err("Title must not be empty".into());
    }
    let (gid, token) = gitlab_context_for_project(&state, project_id).await?;
    let iid = gitlab::create_issue(&state.http, gid, &token, &title, &description).await?;
    refresh_after_write(&state, project_id).await;
    Ok(iid.to_string())
}

/// Add and remove labels on a GitLab issue in a single request. The write is
/// differential, so labels this app does not understand (`component:*`, `tags:*`,
/// a project's own) are left untouched — which is why this does not send `labels`
/// and replace the whole set. The cached row is updated locally too, so the
/// pills change before the refresh comes back.
#[tauri::command]
pub async fn set_issue_labels(
    state: State<'_, AppState>,
    issue_id: i64,
    add: Vec<String>,
    remove: Vec<String>,
) -> Result<(), String> {
    if add.is_empty() && remove.is_empty() {
        return Ok(());
    }
    gitlab::check_label_names(&add, "adding")?;
    gitlab::check_label_names(&remove, "removing")?;
    let (gid, iid, token) = gitlab_context_for_issue(&state, issue_id).await?;
    gitlab::set_labels(&state.http, gid, &iid, &token, &add, &remove).await?;
    {
        let conn = state.db.lock().unwrap();
        let current = se(db::get_issue(&conn, issue_id))?.ok_or("Issue not found")?.labels;
        let next = gitlab::apply_label_change(&current, &add, &remove);
        se(db::set_issue_labels(&conn, issue_id, &next))?;
    }
    refresh_after_write(&state, project_id_for_issue(&state, issue_id)?).await;
    Ok(())
}

/// The labels a project offers, for the picker and its autocompletion. Cached in
/// SQLite because the endpoint costs a request and changes rarely; `refresh`
/// re-reads it, which is what a newly created label needs.
#[tauri::command]
pub async fn get_project_labels(
    state: State<'_, AppState>,
    project_id: i64,
    refresh: bool,
) -> Result<Vec<crate::models::Label>, String> {
    let key = {
        let conn = state.db.lock().unwrap();
        let project = se(db::get_project(&conn, project_id))?.ok_or("Project not found")?;
        if project.kind != "gitlab" {
            return Err(
                "Labels are a GitLab feature; drupal.org issue queues have none.".to_string(),
            );
        }
        project.key
    };
    if !refresh {
        let cached = {
            let conn = state.db.lock().unwrap();
            se(db::list_project_labels(&conn, project_id))?
        };
        if !cached.is_empty() {
            return Ok(cached);
        }
    }
    let token = get_pat()?.ok_or_else(|| {
        "No GitLab token configured. Add a personal access token in Settings to read a project's labels."
            .to_string()
    })?;
    let (gid, _) = gitlab::resolve_project(&state.http, &key, Some(&token)).await?;
    let labels = gitlab::fetch_labels(&state.http, gid, &token).await?;
    {
        let mut conn = state.db.lock().unwrap();
        se(db::replace_project_labels(&mut conn, project_id, &labels))?;
    }
    Ok(labels)
}

#[tauri::command]
pub async fn set_issue_state(
    state: State<'_, AppState>,
    issue_id: i64,
    close: bool,
) -> Result<(), String> {
    let (gid, iid, token) = gitlab_context_for_issue(&state, issue_id).await?;
    let event = if close { "close" } else { "reopen" };
    gitlab::update_issue(&state.http, gid, &iid, &token, None, None, Some(event)).await?;
    refresh_after_write(&state, project_id_for_issue(&state, issue_id)?).await;
    Ok(())
}
