use crate::models::{AssetLink, FetchedComment, FetchedIssue, Label};
use serde_json::{json, Value};

pub const GITLAB_API: &str = "https://git.drupalcode.org/api/v4";

fn auth_headers(token: Option<&str>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(t) = token {
        if let Ok(v) = reqwest::header::HeaderValue::from_str(t) {
            headers.insert("PRIVATE-TOKEN", v);
        }
    }
    headers
}

/// Resolve a GitLab project path like "project/tmgmt_deepl" to (id, name).
pub async fn resolve_project(
    client: &reqwest::Client,
    path: &str,
    token: Option<&str>,
) -> Result<(i64, String), String> {
    let encoded = path.replace('/', "%2F");
    let url = format!("{GITLAB_API}/projects/{encoded}");
    let resp: Value = crate::net::send_retry(client.get(&url).headers(auth_headers(token)))
        .await?
        .json()
        .await
        .map_err(|e| format!("git.drupalcode.org returned invalid JSON: {}", crate::drupal::err_chain(&e)))?;
    if let Some(msg) = resp["message"].as_str() {
        return Err(format!("GitLab error: {msg}"));
    }
    let id = resp["id"].as_i64().ok_or("Unexpected GitLab response")?;
    let name = resp["name_with_namespace"]
        .as_str()
        .unwrap_or(path)
        .to_string();
    Ok((id, name))
}

pub async fn fetch_issues(
    client: &reqwest::Client,
    project_id: i64,
    token: Option<&str>,
    page: usize,
) -> Result<(Vec<FetchedIssue>, bool), String> {
    let url = format!(
        "{GITLAB_API}/projects/{project_id}/issues?order_by=updated_at&sort=desc&per_page=100&page={page}"
    );
    let resp: Value = crate::net::send_retry(client.get(&url).headers(auth_headers(token)))
        .await?
        .json()
        .await
        .map_err(|e| format!("git.drupalcode.org returned invalid JSON: {}", crate::drupal::err_chain(&e)))?;
    let list = resp
        .as_array()
        .ok_or_else(|| "Unexpected GitLab response".to_string())?;

    let issues = list
        .iter()
        .map(|n| {
            let state = n["state"].as_str().unwrap_or("opened");
            let mut status = match state {
                "closed" => "Closed".to_string(),
                "opened" => "Open".to_string(),
                other => other.to_string(),
            };
            let labels: Vec<String> = n["labels"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|l| l.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let scoped = read_labels(&labels);
            if let Some(s) = scoped.status {
                status = s;
            }
            let category = scoped.category.unwrap_or_default();
            let version = scoped.version.unwrap_or_default();
            FetchedIssue {
                ext_id: n["iid"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                title: n["title"].as_str().unwrap_or("").to_string(),
                url: n["web_url"].as_str().unwrap_or("").to_string(),
                status_label: status,
                category_label: category,
                version,
                labels,
                author: n["author"]["username"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                created_at: n["created_at"]
                    .as_str()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| t.timestamp())
                    .unwrap_or(0),
                changed_at: n["updated_at"]
                    .as_str()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| t.timestamp())
                    .unwrap_or(0),
                body: n["description"].as_str().unwrap_or("").to_string(),
                comment_count: n["user_notes_count"].as_i64().unwrap_or(0),
            }
        })
        .collect();
    let has_more = list.len() == 100;
    Ok((issues, has_more))
}

/// Fetch the notes (comments) of an issue, oldest first, walking the pages of
/// the thread. The notes API on git.drupalcode.org requires authentication even
/// for public projects.
///
/// The second field says whether the thread was read to the end: GitLab hands
/// out 100 notes per page, so a short page is the end of the list. Only a
/// complete read may be used to prune cached notes that vanished upstream.
pub async fn fetch_comments(
    client: &reqwest::Client,
    project_id: i64,
    iid: &str,
    token: Option<&str>,
) -> Result<(Vec<FetchedComment>, bool), String> {
    const PAGE_SIZE: usize = 100;
    // Bound the requests: 1000 notes on one issue is already an outlier, and
    // an endless thread must not turn opening it into an unbounded crawl.
    const MAX_PAGES: usize = 10;

    let mut all: Vec<FetchedComment> = Vec::new();
    let mut complete = false;
    for page in 1..=MAX_PAGES {
        let url = format!(
            "{GITLAB_API}/projects/{project_id}/issues/{iid}/notes?sort=asc&order_by=created_at&per_page={PAGE_SIZE}&page={page}"
        );
        let resp: Value = crate::net::send_retry(client.get(&url).headers(auth_headers(token)))
            .await?
            .json()
            .await
            .map_err(|e| format!("git.drupalcode.org returned invalid JSON: {}", crate::drupal::err_chain(&e)))?;
        if let Some(msg) = resp["message"].as_str() {
            return Err(format!(
                "GitLab error: {msg} (reading comments requires a personal access token in Settings)"
            ));
        }
        let list = resp
            .as_array()
            .ok_or_else(|| "Unexpected GitLab notes response".to_string())?;
        let count = list.len();
        all.extend(list.iter().map(|n| FetchedComment {
            ext_id: n["id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
            author: n["author"]["username"].as_str().unwrap_or("").to_string(),
            body: n["body"].as_str().unwrap_or("").to_string(),
            created_at: n["created_at"]
                .as_str()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.timestamp())
                .unwrap_or(0),
            system: n["system"].as_bool().unwrap_or(false),
        }));
        if count < PAGE_SIZE {
            complete = true;
            break;
        }
    }
    Ok((all, complete))
}

/// The drupal.org fields that a GitLab issue carries in its labels.
///
/// Both spellings are read: the form the first migrations used
/// (`"Status: Needs review"`) and GitLab's scoped form, which separates key and
/// value with a *double* colon (`"state::needsReview"`). Values are humanised
/// back to the drupal.org wording so status pills, their colours and the column
/// filters keep working whatever spelling a project uses.
#[derive(Default, Debug, PartialEq)]
pub struct ScopedLabels {
    pub status: Option<String>,
    pub category: Option<String>,
    pub version: Option<String>,
}

pub fn read_labels(labels: &[String]) -> ScopedLabels {
    let mut out = ScopedLabels::default();
    for l in labels {
        // The scoped separator is `::`, so try it first: splitting
        // "state::needsWork" on a single colon would leave a ":needsWork" value.
        // Without `::` fall back to one colon, which covers the migrated
        // "Status: Needs review" spelling (and any hand-made single-colon name).
        let (scope, value) = match l.split_once("::") {
            Some((k, v)) => (k, v),
            None => match l.split_once(':') {
                Some((k, v)) => (k, v),
                None => continue,
            },
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match scope.trim().to_ascii_lowercase().as_str() {
            "status" | "state" => out.status = Some(humanise(value)),
            "category" => out.category = Some(humanise(value)),
            "version" => out.version = Some(value.to_string()),
            // priority::*, why::*, component::*, tags::* and a project's own
            // labels have no field of their own; they are shown as labels verbatim.
            _ => {}
        }
    }
    out
}

/// "needsReview" -> "Needs review", "bug" -> "Bug". Values that are already
/// spaced pass through, except the initialisms drupal.org writes in caps.
fn humanise(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower == "rtbc" {
        return "RTBC".to_string();
    }
    if lower == "wontfix" {
        return "Won't fix".to_string();
    }
    let mut out = String::with_capacity(value.len() + 4);
    for (i, c) in value.chars().enumerate() {
        if c.is_uppercase() && i > 0 && !out.ends_with(' ') {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The label set an issue has after an add/remove edit: order preserved, no
/// duplicates, and a name cannot be both added and removed in one go.
pub fn apply_label_change(current: &[String], add: &[String], remove: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(current.len() + add.len());
    for l in current.iter().chain(add.iter()) {
        let l = l.trim();
        if l.is_empty() || remove.iter().any(|r| r.trim() == l) {
            continue;
        }
        if !out.iter().any(|o| o == l) {
            out.push(l.to_string());
        }
    }
    out
}

/// GitLab's `add_labels`/`remove_labels` are comma-separated and the cache
/// stores one name per line, so a name carrying either must never be sent.
pub fn check_label_names(names: &[String], what: &str) -> Result<(), String> {
    for n in names {
        if n.contains(',') || n.contains('\n') {
            return Err(format!(
                "{what} label '{n}' cannot contain a comma or a line break"
            ));
        }
    }
    Ok(())
}

fn join_names(names: &[String], what: &str) -> Result<String, String> {
    check_label_names(names, what)?;
    Ok(names.join(","))
}

/// Add and remove labels in one request. `add_labels`/`remove_labels` are
/// differential, so labels this app does not understand (`component:*`,
/// `tags:*`, a project's own) keep working — unlike `labels`, which replaces the
/// entire set. GitLab creates a project label for a name that does not exist yet.
pub async fn set_labels(
    client: &reqwest::Client,
    project_id: i64,
    iid: &str,
    token: &str,
    add: &[String],
    remove: &[String],
) -> Result<(), String> {
    let url = format!("{GITLAB_API}/projects/{project_id}/issues/{iid}");
    let mut payload = serde_json::Map::new();
    if !add.is_empty() {
        payload.insert("add_labels".into(), json!(join_names(add, "adding")?));
    }
    if !remove.is_empty() {
        payload.insert("remove_labels".into(), json!(join_names(remove, "removing")?));
    }
    if payload.is_empty() {
        return Ok(());
    }
    let resp = crate::net::send_retry(
        client
            .put(&url)
            .headers(auth_headers(Some(token)))
            .json(&Value::Object(payload)),
    )
    .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let hint = if status.as_u16() == 403 {
            " — labeling needs the Planner role or above in this project;\n\
             the documented fallback is a comment containing /do:label <name>"
        } else {
            ""
        };
        return Err(format!("Updating labels failed ({status}){hint}: {text}"));
    }
    Ok(())
}

/// The labels a project offers, for the picker and its autocompletion. Needs a
/// token: like the notes API, this answers 401 on public projects without one.
pub async fn fetch_labels(
    client: &reqwest::Client,
    project_id: i64,
    token: &str,
) -> Result<Vec<Label>, String> {
    const PAGE_SIZE: usize = 100;
    // Group-level labels are included: on git.drupalcode.org every project sits
    // in a group, and a project may draw its scoped labels from there.
    const MAX_PAGES: usize = 5;

    let mut all: Vec<Label> = Vec::new();
    for page in 1..=MAX_PAGES {
        let url = format!(
            "{GITLAB_API}/projects/{project_id}/labels?include_parent_labels=true&per_page={PAGE_SIZE}&page={page}"
        );
        let resp: Value = crate::net::send_retry(client.get(&url).headers(auth_headers(Some(token))))
            .await?
            .json()
            .await
            .map_err(|e| format!(
                "git.drupalcode.org returned invalid JSON: {}",
                crate::drupal::err_chain(&e)
            ))?;
        if let Some(msg) = resp["message"].as_str() {
            return Err(format!(
                "GitLab error: {msg} (reading a project's labels requires a personal access token in Settings)"
            ));
        }
        let list = resp
            .as_array()
            .ok_or_else(|| "Unexpected GitLab labels response".to_string())?;
        let count = list.len();
        all.extend(list.iter().map(|l| Label {
            name: l["name"].as_str().unwrap_or("").to_string(),
            color: l["color"].as_str().unwrap_or("").to_string(),
            description: l["description"].as_str().unwrap_or("").to_string(),
        }));
        if count < PAGE_SIZE {
            break;
        }
    }
    Ok(all)
}
/// Collect merge-request references from an issue description and its
/// comments (GitLab's related-MR API endpoints are not available on
/// git.drupalcode.org, and only MRs linked from the discussion are wanted
/// anyway). No network access — runs on already-cached content.
///
/// GitLab issue descriptions may contain MR references in multiple formats:
/// 1. Full URL: https://git.drupalcode.org/project/-/merge_requests/123
/// 2. Short reference: !123 (GitLab's internal shorthand, resolved to same project)
/// 3. GitLab.com URLs: https://gitlab.com/project/-/merge_requests/123
pub fn extract_mrs(description: &str, comment_bodies: &[String], issue_url: &str) -> Vec<AssetLink> {
    let mut out: Vec<AssetLink> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    
    // Extract the project namespace from the issue URL for constructing short-ref MR URLs
    // Issue URL format: https://git.drupalcode.org/namespace/project/-/issues/123
    // or: https://gitlab.com/namespace/project/-/issues/123
    let project_path = extract_project_path(issue_url);
    
    // Match full URL formats:
    // - https://git.drupalcode.org/namespace/project/-/merge_requests/123
    // - https://gitlab.com/namespace/project/-/merge_requests/123
    let full_url_re = regex::Regex::new(
        r#"https://(?:git\.drupalcode\.org|gitlab\.com)/[A-Za-z0-9._/%~-]+/-/merge_requests/(\d+)"#
    ).expect("full URL regex");
    
    // Match GitLab's short MR reference notation: !123
    // This appears in formatted descriptions where ! is used as shorthand
    let short_ref_re = regex::Regex::new(r#"!(\d+)"#).expect("short ref regex");
    
    for text in std::iter::once(description).chain(comment_bodies.iter().map(|s| s.as_str())) {
        // Extract full URL matches
        for cap in full_url_re.captures_iter(text) {
            let mr_id = &cap[1];
            let url = cap[0].to_string();
            if !seen.insert(url.clone()) {
                continue;
            }
            out.push(AssetLink {
                name: format!("Merge request !{}", mr_id),
                url,
                kind: "mr".into(),
            });
        }
        
        // Extract short reference matches (!123) and construct the URL
        // Use the project path from the issue URL to construct the MR URL
        for cap in short_ref_re.captures_iter(text) {
            let mr_id = &cap[1];
            let url = if let Some(path) = &project_path {
                format!("https://git.drupalcode.org/{}/-/merge_requests/{}", path, mr_id)
            } else {
                format!("https://git.drupalcode.org/-/merge_requests/{}", mr_id)
            };
            if !seen.insert(url.clone()) {
                continue;
            }
            out.push(AssetLink {
                name: format!("Merge request !{}", mr_id),
                url,
                kind: "mr".into(),
            });
        }
    }
    out
}

/// Drop the MR references whose link no longer exists. A bare `!183` is
/// constructed against this issue's own project path, but when the author
/// actually meant *another* project's MR (a cross-project shorthand — e.g.
/// "inherited from display_builder's unmerged MR !183"), the built URL 404s
/// and clicking it shows GitLab's not-found page. One HEAD per reference
/// against the public MR page answers exactly "will this link 404?":
/// 200 → keep, 404 → drop. Transport failures keep the reference: we could
/// not confirm it is broken, and the panel is read-only anyway, so a
/// possibly-dead link beats an empty one.
///
/// `gitlab.com` URLs are kept unconditionally — this app has no gitlab.com
/// client and every MR reference there is a full URL the author wrote.
/// `gitlab.com` URLs are kept unconditionally — this app has no gitlab.com
/// client and every MR reference there is a full URL the author wrote.
fn needs_network_probe(url: &str) -> bool {
    !url.contains("gitlab.com/")
}

pub async fn prune_missing_mrs(
    client: &reqwest::Client,
    mrs: Vec<AssetLink>,
) -> Vec<AssetLink> {
    let mut out = Vec::with_capacity(mrs.len());
    for mr in mrs {
        let exists = !needs_network_probe(&mr.url)
            || mr_ui_url_exists(client, &mr.url).await;
        if exists {
            out.push(mr);
        } else {
            eprintln!("[assets] dropped missing MR reference: {}", mr.url);
        }
    }
    out
}

pub async fn mr_ui_url_exists(client: &reqwest::Client, ui_url: &str) -> bool {
    let resp = match crate::net::send_retry(client.head(ui_url)).await {
        Ok(resp) => resp,
        Err(_) => return true, // transport failure: keep, cannot confirm it is broken
    };
    let status = resp.status();
    // HEAD on the public MR page: 200/302 (redirect to login) = exists,
    // 404 = the reference is dead. Anything else, keep.
    match status.as_u16() {
        200 | 301 | 302 | 303 | 307 | 308 => true,
        404 | 410 => false,
        _ => true,
    }
}

/// Extract the project path from an issue URL.
/// Formats: https://git.drupalcode.org/namespace/project/-/issues/123
///          https://gitlab.com/namespace/project/-/issues/123
/// GitLab work items on git.drupalcode.org use a different URL shape:
/// https://git.drupalcode.org/namespace/project/-/work_items/123
fn extract_project_path(url: &str) -> Option<String> {
    // Try to parse gitlab.com URLs first, then fall back to git.drupalcode.org
    // `work_items` covers GitLab work-item URLs (the shape git.drupalcode.org
    // uses for migrated drupal.org issues); `issues` is the classic shape on
    // both hosts.
    let patterns = [
        r#"https://gitlab\.com/([A-Za-z0-9._/%~]+)/-/(?:issues|work_items)/"#,
        r#"https://git\.drupalcode\.org/([A-Za-z0-9._/%~]+)/-/(?:issues|work_items)/"#,
    ];
    for pattern in &patterns {
        let re = regex::Regex::new(pattern).ok()?;
        if let Some(cap) = re.captures(url) {
            return Some(cap[1].to_string());
        }
    }
    None
}
pub async fn validate_token(client: &reqwest::Client, token: &str) -> Result<String, String> {
    let url = format!("{GITLAB_API}/user");
    let resp: Value = crate::net::send_retry(client.get(&url).headers(auth_headers(Some(token))))
        .await?
        .json()
        .await
        .map_err(|e| format!("invalid response from git.drupalcode.org: {}", crate::drupal::err_chain(&e)))?;
    if let Some(msg) = resp["message"].as_str() {
        return Err(format!("Token rejected: {msg}"));
    }
    resp["username"]
        .as_str()
        .map(|u| u.to_string())
        .ok_or_else(|| "Unexpected response validating token".to_string())
}

pub async fn add_comment(
    client: &reqwest::Client,
    project_id: i64,
    iid: &str,
    token: &str,
    body: &str,
) -> Result<(), String> {
    let url = format!("{GITLAB_API}/projects/{project_id}/issues/{iid}/notes");
    let resp = crate::net::send_retry(
        client
            .post(&url)
            .headers(auth_headers(Some(token)))
            .json(&json!({ "body": body })),
    )
    .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Adding comment failed ({status}): {text}"));
    }
    Ok(())
}

/// Note ids come from the API/cache rather than typed input, but the path
/// segment is validated anyway so a malformed row cannot shape a URL.
fn note_path(project_id: i64, iid: &str, note_id: &str) -> Result<String, String> {
    let id = note_id
        .parse::<i64>()
        .map_err(|_| format!("Unexpected GitLab comment id '{note_id}'"))?;
    Ok(format!(
        "{GITLAB_API}/projects/{project_id}/issues/{iid}/notes/{id}"
    ))
}

/// Change the body of an existing note. GitLab allows only the note's author
/// (or an instance admin), so the caller has to gate the UI on it.
pub async fn edit_comment(
    client: &reqwest::Client,
    project_id: i64,
    iid: &str,
    note_id: &str,
    token: &str,
    body: &str,
) -> Result<(), String> {
    let url = note_path(project_id, iid, note_id)?;
    let resp = crate::net::send_retry(
        client
            .put(&url)
            .headers(auth_headers(Some(token)))
            .json(&json!({ "body": body })),
    )
    .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Updating comment failed ({status}): {text}"));
    }
    Ok(())
}

/// Delete a note for everyone. Author-only, like the edit above.
pub async fn delete_comment(
    client: &reqwest::Client,
    project_id: i64,
    iid: &str,
    note_id: &str,
    token: &str,
) -> Result<(), String> {
    let url = note_path(project_id, iid, note_id)?;
    let resp =
        crate::net::send_retry(client.delete(&url).headers(auth_headers(Some(token)))).await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Deleting comment failed ({status}): {text}"));
    }
    Ok(())
}

pub async fn update_issue(
    client: &reqwest::Client,
    project_id: i64,
    iid: &str,
    token: &str,
    title: Option<&str>,
    description: Option<&str>,
    state_event: Option<&str>, // "close" | "reopen"
) -> Result<(), String> {
    let url = format!("{GITLAB_API}/projects/{project_id}/issues/{iid}");
    let mut payload = serde_json::Map::new();
    if let Some(t) = title {
        payload.insert("title".into(), json!(t));
    }
    if let Some(d) = description {
        payload.insert("description".into(), json!(d));
    }
    if let Some(s) = state_event {
        payload.insert("state_event".into(), json!(s));
    }
    let resp = crate::net::send_retry(
        client
            .put(&url)
            .headers(auth_headers(Some(token)))
            .json(&Value::Object(payload)),
    )
    .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Updating issue failed ({status}): {text}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ls(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn scoped_and_legacy_labels_both_fill_the_same_fields() {
        assert_eq!(
            read_labels(&ls(&[
                "category::bug",
                "state::needsReview",
                "version::11.x-dev"
            ])),
            ScopedLabels {
                status: Some("Needs review".into()),
                category: Some("Bug".into()),
                version: Some("11.x-dev".into()),
            }
        );
        assert_eq!(
            read_labels(&ls(&[
                "Status: Needs review",
                "Category: Bug",
                "Version: 10.5.x-dev"
            ])),
            ScopedLabels {
                status: Some("Needs review".into()),
                category: Some("Bug".into()),
                version: Some("10.5.x-dev".into()),
            }
        );
    }

    #[test]
    fn labels_outside_the_known_scopes_are_left_alone() {
        assert_eq!(
            read_labels(&ls(&[
                "priority::major",
                "why::duplicate",
                "component::forms",
                "untagged"
            ])),
            ScopedLabels::default()
        );
    }

    #[test]
    fn a_scoped_value_may_still_carry_a_single_colon() {
        assert_eq!(
            read_labels(&ls(&["version::11.x:dev"])),
            ScopedLabels {
                version: Some("11.x:dev".into()),
                ..Default::default()
            }
        );
    }

    #[test]
    fn drupal_org_wording_survives_the_humaniser() {
        assert_eq!(humanise("rtbc"), "RTBC");
        assert_eq!(humanise("toBePorted"), "To be ported");
        assert_eq!(humanise("needsWork"), "Needs work");
        assert_eq!(humanise("bug"), "Bug");
        // Values that are already spaced must not gain a space per capital, but
        // keep their leading capital for the pill.
        assert_eq!(humanise("closed (duplicate)"), "Closed (duplicate)");
    }

    #[test]
    fn a_label_edit_keeps_unknown_labels_and_order() {
        assert_eq!(
            apply_label_change(
                &ls(&["category::bug", "component::forms", "state::needsWork"]),
                &ls(&["state::needsReview", "priority::major"]),
                &ls(&["state::needsWork"])
            ),
            ls(&[
                "category::bug",
                "component::forms",
                "state::needsReview",
                "priority::major"
            ]),
            "the label we do not understand survives, the replaced one does not"
        );
    }

    #[test]
    fn a_name_with_a_comma_is_refused_before_it_reaches_the_api() {
        assert!(join_names(&ls(&["one,two"]), "adding").is_err());
        assert_eq!(
            join_names(&ls(&["category::bug"]), "adding").unwrap(),
            "category::bug"
        );
    }

    #[test]
    fn extracts_mrs_from_full_urls() {
        let desc = "Related: https://git.drupalcode.org/project/-/merge_requests/42";
        let mrs = extract_mrs(desc, &[], "https://git.drupalcode.org/project/-/issues/1");
        assert_eq!(mrs.len(), 1);
        assert_eq!(mrs[0].name, "Merge request !42");
        assert_eq!(mrs[0].url, "https://git.drupalcode.org/project/-/merge_requests/42");
    }

    #[test]
    fn extracts_mrs_from_short_reference() {
        let desc = "Closes !42 and !43";
        let mrs = extract_mrs(desc, &[], "https://git.drupalcode.org/myproject/-/issues/1");
        assert!(!mrs.is_empty());
    }

    #[test]
    fn short_reference_uses_the_work_item_issue_url() {
        // git.drupalcode.org stores drupal.org imports as work items:
        // …/project/ui_patterns_library_plus/-/work_items/11 — the project
        // path must be extracted from that shape too, or !8 falls back to
        // the account-wide https://git.drupalcode.org/-/merge_requests/8.
        let desc = "See !8";
        let mrs = extract_mrs(
            desc,
            &[],
            "https://git.drupalcode.org/project/ui_patterns_library_plus/-/work_items/11",
        );
        assert_eq!(mrs.len(), 1);
        assert_eq!(
            mrs[0].url,
            "https://git.drupalcode.org/project/ui_patterns_library_plus/-/merge_requests/8"
        );
    }

    #[test]
    fn handles_gitlab_com_urls() {
        let desc = "MR: https://gitlab.com/namespace/project/-/merge_requests/123";
        let mrs = extract_mrs(desc, &[], "https://gitlab.com/namespace/project/-/issues/1");
        assert_eq!(mrs.len(), 1);
        assert_eq!(mrs[0].url, "https://gitlab.com/namespace/project/-/merge_requests/123");
    }

    #[test]
    fn deduplicates_mr_urls() {
        let desc = "Related: https://git.drupalcode.org/project/-/merge_requests/42"
            .to_string() + " and another: https://git.drupalcode.org/project/-/merge_requests/42";
        let mrs = extract_mrs(&desc, &[], "https://git.drupalcode.org/project/-/issues/1");
        assert_eq!(mrs.len(), 1);
    }

    #[test]
    fn gitlab_com_references_are_kept_without_a_network_probe() {
        // gitlab.com links are kept unconditionally (this app has no
        // gitlab.com client); drupalcode links get probed.
        assert!(!needs_network_probe("https://gitlab.com/namespace/project/-/merge_requests/123"));
        assert!(needs_network_probe(
            "https://git.drupalcode.org/project/ui_patterns_library_plus/-/merge_requests/8"
        ));
    }

    #[test]
    fn extracts_mrs_from_comments() {
        let comment1 = "Check this out: !42".to_string();
        let comment2 = "See https://git.drupalcode.org/project/-/merge_requests/99".to_string();
        let mrs = extract_mrs("", &[comment1, comment2], "https://git.drupalcode.org/project/-/issues/1");
        assert!(!mrs.is_empty());
    }


}
