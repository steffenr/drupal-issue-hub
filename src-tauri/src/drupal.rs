use crate::models::{AssetLink, FetchedComment, FetchedIssue};
use serde_json::Value;

/// Format an error with its full source chain (reqwest hides the real cause
/// in its Display output).
pub fn err_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(inner) = src {
        s.push_str(&format!(" | caused by: {inner}"));
        src = inner.source();
    }
    s
}

pub const DRUPAL_API: &str = "https://www.drupal.org/api-d7";

pub fn status_label(code: &str) -> String {
    match code {
        "1" => "Active",
        "2" => "Fixed",
        "3" => "Closed (duplicate)",
        "4" => "Postponed",
        "5" => "Closed (won't fix)",
        "6" => "Closed (works as designed)",
        "7" => "Closed (fixed)",
        "8" => "Needs review",
        "13" => "Needs work",
        "14" => "RTBC",
        "15" => "Patch (to be ported)",
        "16" => "Postponed (maintainer needs more info)",
        "17" => "Closed (outdated)",
        "18" => "Closed (cannot reproduce)",
        other => return format!("Status {other}"),
    }
    .to_string()
}

pub fn category_label(code: &str) -> String {
    match code {
        "1" => "Bug",
        "2" => "Task",
        "3" => "Feature",
        "4" => "Support",
        "5" => "Plan",
        other => return format!("Category {other}"),
    }
    .to_string()
}

fn s(v: &Value, key: &str) -> String {
    match &v[key] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Look up a project by machine name (e.g. "paragraphs") and return (nid, title).
pub async fn resolve_project(
    client: &reqwest::Client,
    machine_name: &str,
) -> Result<(String, String), String> {
    let url = format!("{DRUPAL_API}/node.json?field_project_machine_name={machine_name}&limit=1");
    let resp: Value = crate::net::send_retry(client.get(&url))
        .await?
        .error_for_status()
        .map_err(|e| format!("drupal.org rejected the project lookup: {e}"))?
        .json()
        .await
        .map_err(|e| format!("drupal.org returned invalid JSON: {}", err_chain(&e)))?;
    let first = resp["list"]
        .as_array()
        .and_then(|l| l.first())
        .ok_or_else(|| format!("No drupal.org project found for machine name '{machine_name}'"))?;
    Ok((s(first, "nid"), s(first, "title")))
}

/// Page size the drupal.org API actually serves (it silently caps `limit`
/// at 50 per page; `page` is 0-based).
pub const ISSUES_PER_PAGE: usize = 50;

pub async fn fetch_issues(
    client: &reqwest::Client,
    project_nid: &str,
    page: usize,
) -> Result<(Vec<FetchedIssue>, bool), String> {
    let url = format!(
        "{DRUPAL_API}/node.json?type=project_issue&field_project={project_nid}\
         &sort=changed&direction=DESC&limit={ISSUES_PER_PAGE}&page={page}"
    );
    let resp: Value = crate::net::send_retry(client.get(&url))
        .await?
        .error_for_status()
        .map_err(|e| format!("drupal.org rejected the issue query: {e}"))?
        .json()
        .await
        .map_err(|e| format!("drupal.org returned invalid JSON: {}", err_chain(&e)))?;

    let list = resp["list"]
        .as_array()
        .ok_or_else(|| "Unexpected drupal.org API response".to_string())?;

    let issues = list
        .iter()
        .map(|n| FetchedIssue {
            ext_id: s(n, "nid"),
            title: s(n, "title"),
            url: s(n, "url"),
            status_label: status_label(&s(n, "field_issue_status")),
            category_label: category_label(&s(n, "field_issue_category")),
            version: s(n, "field_issue_version"),
            // drupal.org issue nodes include author.name, but some may not.
            author: n["author"]["name"].as_str().unwrap_or_else(|| {
                n["author"]["id"].as_str().unwrap_or("")
            }).to_string(),
            created_at: s(n, "created").parse().unwrap_or(0),
            changed_at: s(n, "changed").parse().unwrap_or(0),
            comment_count: s(n, "comment_count").parse().unwrap_or(0),
            body: n["body"]["value"].as_str().unwrap_or_default().to_string(),
            labels: Vec::new(),
        })
        .collect();
    let has_more = list.len() == ISSUES_PER_PAGE;
    Ok((issues, has_more))
}

/// Resolve an author UID to a human-readable username by fetching user.json.
/// drupal.org stores comment author as a user reference `{id: uid}` without the name field.
async fn resolve_author_name(client: &reqwest::Client, uid: &str) -> Option<String> {
    if uid.is_empty() || uid == "0" {
        return None;
    }
    let url = format!("{DRUPAL_API}/user.json?uid={uid}&limit=1");
    let Ok(resp) = crate::net::send_retry(client.get(&url)).await else {
        return None;
    };
    let Ok(resp) = resp.error_for_status() else {
        return None;
    };
    let Ok(resp) = resp.json::<Value>().await else {
        return None;
    };
    resp["list"]
        .as_array()
        .and_then(|l| l.first())
        .and_then(|u| u["name"].as_str())
        .map(|s| s.to_string())
}

/// Fetch a single issue node: its current fields (for edit detection while
/// viewing) plus the ids of its comments, newest first.
pub async fn fetch_issue_node(
    client: &reqwest::Client,
    issue_nid: &str,
) -> Result<(FetchedIssue, Vec<String>), String> {
    let url = format!("{DRUPAL_API}/node.json?nid={issue_nid}");
    let resp: Value = crate::net::send_retry(client.get(&url))
        .await?
        .error_for_status()
        .map_err(|e| format!("drupal.org rejected the issue lookup: {e}"))?
        .json()
        .await
        .map_err(|e| format!("drupal.org returned invalid JSON: {}", err_chain(&e)))?;
    let node = resp["list"]
        .as_array()
        .and_then(|l| l.first())
        .ok_or_else(|| format!("drupal.org issue {issue_nid} not found"))?;
    let ids = node["comments"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c["id"].as_i64().map(|v| v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let author = node["author"]["name"].as_str()
        .or_else(|| node["author"]["id"].as_str())
        .unwrap_or("")
        .to_string();
    Ok((FetchedIssue {
        ext_id: s(node, "nid"),
        title: s(node, "title"),
        url: s(node, "url"),
        status_label: status_label(&s(node, "field_issue_status")),
        category_label: category_label(&s(node, "field_issue_category")),
        version: s(node, "field_issue_version"),
        author,
        created_at: s(node, "created").parse().unwrap_or(0),
        changed_at: s(node, "changed").parse().unwrap_or(0),
        comment_count: s(node, "comment_count").parse().unwrap_or(0),
        body: node["body"]["value"].as_str().unwrap_or_default().to_string(),
        labels: Vec::new(),
    }, ids))
}

/// Fetch a single drupal.org comment by cid. Comment bodies are filtered HTML.
pub async fn fetch_comment(
    client: &reqwest::Client,
    cid: &str,
) -> Result<Option<FetchedComment>, String> {
    let url = format!("{DRUPAL_API}/comment.json?cid={cid}");
    let resp: Value = crate::net::send_retry(client.get(&url))
        .await?
        .error_for_status()
        .map_err(|e| format!("drupal.org rejected the comment lookup: {e}"))?
        .json()
        .await
        .map_err(|e| format!("drupal.org returned invalid JSON: {}", err_chain(&e)))?;
    let Some(n) = resp["list"].as_array().and_then(|l| l.first()) else {
        return Ok(None);
    };
    let uid = n["author"]["id"].as_str().unwrap_or("").to_string();
    let author = resolve_author_name(client, &uid).await.unwrap_or(uid);
    Ok(Some(FetchedComment {
        ext_id: s(n, "cid"),
        author,
        body: n["comment_body"]["value"].as_str().unwrap_or_default().to_string(),
        created_at: s(n, "created").parse().unwrap_or(0),
        system: false,
    }))
}

/// Search d.o projects by machine name (exact first) or title (prefix match).
pub async fn search_projects(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<crate::models::ProjectSuggestion>, String> {
    let mut out: Vec<crate::models::ProjectSuggestion> = Vec::new();
    if let Ok((nid, title)) = resolve_project(client, query).await {
        out.push(crate::models::ProjectSuggestion {
            id: nid,
            kind: "drupal".into(),
            title,
            add_url: format!("https://www.drupal.org/project/{query}"),
        });
    }
    let url = format!(
        "{DRUPAL_API}/node.json?type=project_module&title={}&limit=5",
        query.replace(' ', "+")
    );
    if let Ok(resp) = client.get(&url).send().await {
        if let Ok(resp) = resp.json::<Value>().await {
            if let Some(list) = resp["list"].as_array() {
                for n in list {
                    let nid = s(n, "nid");
                    if out.iter().any(|p| p.id == nid) {
                        continue;
                    }
                    let title = s(n, "title");
                    let machine = s(n, "field_project_machine_name");
                    let machine = if machine.is_empty() {
                        title.to_lowercase().replace(' ', "_")
                    } else {
                        machine
                    };
                    out.push(crate::models::ProjectSuggestion {
                        id: nid,
                        kind: "drupal".into(),
                        title,
                        add_url: format!("https://www.drupal.org/project/{machine}"),
                    });
                    if out.len() >= 6 {
                        break;
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Extract patch files and merge-request links from an issue page.
pub async fn fetch_issue_assets(
    client: &reqwest::Client,
    issue_url: &str,
) -> Result<Vec<AssetLink>, String> {
    let parsed = reqwest::Url::parse(issue_url)
        .map_err(|e| format!("invalid issue URL '{issue_url}': {e}"))?;
    let host_ok = matches!(
        parsed.host_str(),
        Some("www.drupal.org") | Some("drupal.org")
    );
    if parsed.scheme() != "https" || !host_ok {
        return Err(format!(
            "Refusing to fetch issue page from non-drupal.org URL: {issue_url}"
        ));
    }
    let html = crate::net::send_retry(client.get(issue_url))
        .await?
        .error_for_status()
        .map_err(|e| format!("could not load the issue page: {e}"))?
        .text()
        .await
        .map_err(|e| format!("could not read the issue page: {e}"))?;
    Ok(extract_assets(&html))
}

fn extract_assets(html: &str) -> Vec<AssetLink> {
    let mut out: Vec<AssetLink> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let file_re =
        regex::Regex::new(r#"<a\s[^>]*href="(https://www\.drupal\.org/files/issues/[^"]+)"(?:\s+type="([^"]*)")?[^>]*>([^<]+)</a>"#)
            .expect("file regex");
    for cap in file_re.captures_iter(html) {
        let url = cap[1].to_string();
        let mime = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let name = html_unescape(&cap[3]);
        let lower_url = url.to_lowercase();
        let name_lower = name.to_lowercase();
        let is_patch = mime.contains("x-diff")
            || lower_url.ends_with(".patch")
            || lower_url.ends_with(".diff")
            || name_lower.ends_with(".patch")
            || name_lower.ends_with(".diff");
        if !is_patch || !seen.insert(url.clone()) {
            continue;
        }
        out.push(AssetLink {
            name,
            url,
            kind: "patch".into(),
        });
    }
    let mr_re = regex::Regex::new(
        r#"https://git\.drupalcode\.org/[A-Za-z0-9._/%~-]+/-/merge_requests/(\d+)"#,
    )
    .expect("mr regex");
    for cap in mr_re.captures_iter(html) {
        let url = cap[0].to_string();
        if !seen.insert(url.clone()) {
            continue;
        }
        out.push(AssetLink {
            name: format!("Merge request !{}", &cap[1]),
            url,
            kind: "mr".into(),
        });
    }
    out
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_patches_and_mrs() {
        let html = r#"
            <a href="https://www.drupal.org/files/issues/2024-01-01/fix-123-45.patch" type="text/x-diff; length=1000">fix-123-45.patch</a>
            <a href="https://www.drupal.org/files/issues/2024-01-01/fix-123-45.patch" type="text/x-diff; length=1000">fix-123-45.patch</a>
            <a href="https://www.drupal.org/files/issues/2026-09-09/image%20%281%29.png" type="image/png; length=595041">image (1).png</a>
            <a href="https://git.drupalcode.org/project/paragraphs/-/merge_requests/207">!207</a>
            <a href="/relative/not-a-file.txt">nope</a>
        "#;
        let assets = extract_assets(html);
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].kind, "patch");
        assert_eq!(assets[0].name, "fix-123-45.patch");
        assert_eq!(assets[1].kind, "mr");
        assert!(assets[1].url.ends_with("/merge_requests/207"));
    }
}
