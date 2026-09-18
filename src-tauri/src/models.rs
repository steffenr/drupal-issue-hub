use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Project {
    pub id: i64,
    pub kind: String, // "drupal" | "gitlab"
    pub key: String,  // drupal: numeric project nid; gitlab: url path e.g. "project/tmgmt_deepl"
    pub name: String,
    pub url: String,
    pub last_refreshed: Option<i64>,
    pub last_error: Option<String>,
    pub new_count: i64,
    pub changed_count: i64,
    pub issue_count: i64,
    /// Per-module notification switches, both honoured only by the poll.
    pub notify_new_issues: bool,
    pub notify_new_comments: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub project_name: String,
    pub source: String, // "drupal" | "gitlab"
    pub ext_id: String, // drupal: issue nid; gitlab: issue iid
    pub title: String,
    pub url: String,
    pub status_label: String,
    pub category_label: String,
    pub version: String,
    pub author: String,
    pub created_at: i64,
    pub changed_at: i64,
    pub body: String,
    pub comment_count: i64,
    /// GitLab labels, in their stored order. drupal.org has no write API, so
    /// these are only ever set for gitlab rows.
    pub labels: Vec<String>,
    pub favorite: bool,
    pub is_new: bool,
    pub has_changes: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Comment {
    pub ext_id: String,
    pub author: String,
    pub body: String,
    pub created_at: i64,
    pub system: bool,
    pub is_new: bool,
}

#[derive(Clone, Debug)]
pub struct FetchedComment {
    pub ext_id: String,
    pub author: String,
    pub body: String,
    pub created_at: i64,
    pub system: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct AssetLink {
    pub name: String,
    pub url: String,
    pub kind: String, // "patch" | "mr"
}

#[derive(Serialize, Clone, Debug)]
pub struct LoadMoreResult {
    pub loaded: i64,
    pub has_more: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct RefreshResult {
    pub project_id: i64,
    pub project_name: String,
    pub ok: bool,
    pub error: Option<String>,
    pub new_issues: i64,
    pub changed_issues: i64,
    /// Comments added to issues this project already had (count deltas).
    pub new_comments: i64,
    /// Newest issue this sync found news in, for following a notification.
    pub target_issue_id: Option<i64>,
    /// The project's switches, so the caller decides what is worth reporting.
    pub notify_new_issues: bool,
    pub notify_new_comments: bool,
}

#[derive(Clone, Debug)]
pub struct FetchedIssue {
    pub ext_id: String,
    pub title: String,
    pub url: String,
    pub status_label: String,
    pub category_label: String,
    pub version: String,
    pub author: String,
    pub created_at: i64,
    pub changed_at: i64,
    pub body: String,
    pub comment_count: i64,
    /// Every label of the issue, as GitLab returns it, including the ones this
    /// app does not interpret — they must survive the round trip so a label edit
    /// cannot wipe them.
    pub labels: Vec<String>,
}

/// One label of a GitLab project, as the labels API returns it. Cached per
/// project to drive the picker and its autocompletion.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Label {
    pub name: String,
    pub color: String,
    pub description: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Settings {
    pub poll_enabled: bool,
    pub poll_interval_minutes: u64,
    /// Hide the host's "Status changed to …" notes in the thread. Display only:
    /// the cache keeps them and notification counts are unaffected.
    pub hide_system_comments: bool,
    /// Table filter a freshly opened project starts in: "attention" or "all".
    pub default_mode: String,
    pub gitlab_user: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectSuggestion {
    pub id: String,      // drupal nid or gitlab path
    pub kind: String,
    pub title: String,
    pub add_url: String, // what to pass to add_project
}
