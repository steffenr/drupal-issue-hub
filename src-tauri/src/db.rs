use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Create the tables and apply the additive column migrations. Split out of
/// `open` so tests can bring an in-memory database up to the current schema.
pub fn init_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS projects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            key TEXT NOT NULL,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            last_refreshed INTEGER,
            last_error TEXT,
            UNIQUE(kind, key)
         );
         CREATE TABLE IF NOT EXISTS issues (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            ext_id TEXT NOT NULL,
            title TEXT NOT NULL,
            url TEXT NOT NULL DEFAULT '',
            status_label TEXT NOT NULL DEFAULT '',
            category_label TEXT NOT NULL DEFAULT '',
            version TEXT NOT NULL DEFAULT '',
            author TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0,
            changed_at INTEGER NOT NULL DEFAULT 0,
            first_seen INTEGER NOT NULL DEFAULT 0,
            seen INTEGER NOT NULL DEFAULT 1,
            seen_changed INTEGER NOT NULL DEFAULT 1,
            UNIQUE(project_id, ext_id)
         );
         CREATE INDEX IF NOT EXISTS idx_issues_project ON issues(project_id, changed_at);
         CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
         );",
    )?;
    // Migration: comment_count was added after the first release.
    let has_comment_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('issues') WHERE name = 'comment_count'",
        [],
        |r| r.get(0),
    )?;
    if has_comment_count == 0 {
        conn.execute(
            "ALTER TABLE issues ADD COLUMN comment_count INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    // Migration: favorites were added after the first release.
    let has_favorite: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('issues') WHERE name = 'favorite'",
        [],
        |r| r.get(0),
    )?;
    if has_favorite == 0 {
        conn.execute(
            "ALTER TABLE issues ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    // Migration: manual project ordering was added after the first release.
    let has_sort_order: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'sort_order'",
        [],
        |r| r.get(0),
    )?;
    if has_sort_order == 0 {
        conn.execute(
            "ALTER TABLE projects ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        conn.execute("UPDATE projects SET sort_order = id WHERE sort_order = 0", [])?;
    }
    // Migration: number of issue pages already fetched for lazy loading.
    let has_pages_fetched: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'pages_fetched'",
        [],
        |r| r.get(0),
    )?;
    if has_pages_fetched == 0 {
        conn.execute(
            "ALTER TABLE projects ADD COLUMN pages_fetched INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    // Migration: per-project notification switches. Both default to on — a
    // project you follow should report both signals — and each module can turn
    // either one off from its row in the sidebar.
    for col in ["notify_new_issues", "notify_new_comments"] {
        let has: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = '{col}'"),
            [],
            |r| r.get(0),
        )?;
        if has == 0 {
            conn.execute(
                &format!("ALTER TABLE projects ADD COLUMN {col} INTEGER NOT NULL DEFAULT 1"),
                [],
            )?;
        }
    }
    // Migration: the GitLab label set of an issue, needed both to show what an
    // issue carries and to compute the add/remove diff a label edit sends.
    let has_labels: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('issues') WHERE name = 'labels'",
        [],
        |r| r.get(0),
    )?;
    if has_labels == 0 {
        conn.execute(
            "ALTER TABLE issues ADD COLUMN labels TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS comments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            issue_id INTEGER NOT NULL,
            ext_id TEXT NOT NULL,
            author TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0,
            system INTEGER NOT NULL DEFAULT 0,
            seen INTEGER NOT NULL DEFAULT 1,
            UNIQUE(issue_id, ext_id)
         );
         CREATE TABLE IF NOT EXISTS project_labels (
            project_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            color TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            fetched_at INTEGER NOT NULL DEFAULT 0,
            UNIQUE(project_id, name)
         );",
    )?;
    Ok(())
}

#[derive(Clone)]
pub struct ProjectRow {
    pub id: i64,
    pub kind: String,
    pub key: String,
    pub name: String,
    pub url: String,
    pub last_refreshed: Option<i64>,
    pub last_error: Option<String>,
    pub new_count: i64,
    pub changed_count: i64,
    pub issue_count: i64,
    pub notify_new_issues: bool,
    pub notify_new_comments: bool,
}

pub fn list_projects(conn: &Connection) -> Result<Vec<ProjectRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.kind, p.key, p.name, p.url, p.last_refreshed, p.last_error,
            (SELECT COUNT(*) FROM issues i WHERE i.project_id = p.id AND i.seen = 0),
            (SELECT COUNT(*) FROM issues i WHERE i.project_id = p.id AND i.seen = 1 AND i.seen_changed = 0),
            (SELECT COUNT(*) FROM issues i WHERE i.project_id = p.id),
            p.notify_new_issues, p.notify_new_comments
         FROM projects p ORDER BY p.sort_order ASC, p.id ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ProjectRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                key: r.get(2)?,
                name: r.get(3)?,
                url: r.get(4)?,
                last_refreshed: r.get(5)?,
                last_error: r.get(6)?,
                new_count: r.get(7)?,
                changed_count: r.get(8)?,
                issue_count: r.get(9)?,
                notify_new_issues: r.get::<_, i64>(10)? != 0,
                notify_new_comments: r.get::<_, i64>(11)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_project(conn: &Connection, id: i64) -> Result<Option<ProjectRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.kind, p.key, p.name, p.url, p.last_refreshed, p.last_error,
            (SELECT COUNT(*) FROM issues i WHERE i.project_id = p.id AND i.seen = 0),
            (SELECT COUNT(*) FROM issues i WHERE i.project_id = p.id AND i.seen = 1 AND i.seen_changed = 0),
            (SELECT COUNT(*) FROM issues i WHERE i.project_id = p.id),
            p.notify_new_issues, p.notify_new_comments
         FROM projects p WHERE p.id = ?1",
    )?;
    let row = stmt
        .query_row([id], |r| {
            Ok(ProjectRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                key: r.get(2)?,
                name: r.get(3)?,
                url: r.get(4)?,
                last_refreshed: r.get(5)?,
                last_error: r.get(6)?,
                new_count: r.get(7)?,
                changed_count: r.get(8)?,
                issue_count: r.get(9)?,
                notify_new_issues: r.get::<_, i64>(10)? != 0,
                notify_new_comments: r.get::<_, i64>(11)? != 0,
            })
        })
        .optional()?;
    Ok(row)
}

pub fn find_project_by_key(
    conn: &Connection,
    kind: &str,
    key: &str,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.query_row(
        "SELECT id FROM projects WHERE kind = ?1 AND key = ?2",
        params![kind, key],
        |r| r.get(0),
    )
    .optional()
}

/// What one sync of a project changed, and what a notification should show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncDelta {
    pub new_issues: i64,
    pub changed_issues: i64,
    pub new_comments: i64,
    /// The most recently changed issue this sync found news in (a new issue,
    /// or one whose comment count rose). Lets a followed notification preselect
    /// an issue; `None` when there is nothing to show.
    pub target_issue_id: Option<i64>,
}

/// Insert or update a batch of fetched issues, reporting a [`SyncDelta`]. On
/// the very first fetch of a project everything is marked as seen and the
/// delta comes back empty — a backlog import is not news, and reporting it
/// would bury the user in "new" flags and a bogus notification. `mark_new=false`
/// (deep-page lazy loads) also keeps historical issues off the NEW badge;
/// callers that are not refreshing (load-more, the single-issue node fetch)
/// ignore the delta.
pub fn upsert_issues(
    conn: &mut Connection,
    project_id: i64,
    issues: &[crate::models::FetchedIssue],
    mark_new: bool,
) -> Result<SyncDelta, rusqlite::Error> {
    let existing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM issues WHERE project_id = ?1",
        [project_id],
        |r| r.get(0),
    )?;
    let initial_sync = existing == 0;
    let insert_seen: i64 = if initial_sync || !mark_new { 1 } else { 0 };
    let now = chrono::Utc::now().timestamp();

    let tx = conn.transaction()?;
    let mut delta = SyncDelta::default();
    // The newest thing we found news in, as (issue id, changed_at, priority).
    // Priority ranks a brand new issue above a comment on an existing one, so
    // the choice never depends on the order rows arrive in.
    let mut target: Option<(i64, i64, u8)> = None;
    let mut note_target = |id: i64, at: i64, prio: u8| {
        if target.map_or(true, |(_, best_at, best_prio)| (at, prio) > (best_at, best_prio)) {
            target = Some((id, at, prio));
        }
    };
    for f in issues {
        let labels = join_labels(&f.labels);
        let prev: Option<(i64, i64, i64)> = tx
            .query_row(
                "SELECT id, changed_at, comment_count FROM issues WHERE project_id = ?1 AND ext_id = ?2",
                params![project_id, f.ext_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        // The comment delta comes from the stored count alone, independent of
        // the changed timestamp: both queues sort by recency of change, so an
        // issue that gained a comment is always inside the page just fetched.
        // Only increases count, and only for issues we already had — a new
        // issue's own opening comments are the discussion at filing, not news.
        if let Some((id, _, prev_comments)) = &prev {
            if f.comment_count > *prev_comments {
                delta.new_comments += f.comment_count - *prev_comments;
                note_target(*id, f.changed_at, 0);
            }
        }
        match prev {
            None => {
                tx.execute(
                    "INSERT INTO issues (project_id, ext_id, title, url, status_label, category_label,
                        version, author, body, comment_count, created_at, changed_at, first_seen, seen, seen_changed, labels)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1, ?15)",
                    params![
                        project_id,
                        f.ext_id,
                        f.title,
                        f.url,
                        f.status_label,
                        f.category_label,
                        f.version,
                        f.author,
                        f.body,
                        f.comment_count,
                        f.created_at,
                        f.changed_at,
                        now,
                        insert_seen,
                        labels
                    ],
                )?;
                delta.new_issues += 1;
                note_target(tx.last_insert_rowid(), f.changed_at, 1);
            }
            Some((_, prev_changed, _)) if prev_changed != f.changed_at => {
                tx.execute(
                    "UPDATE issues SET title = ?3, url = ?4, status_label = ?5, category_label = ?6,
                        version = ?7, author = ?8, body = ?9, comment_count = ?10, changed_at = ?11,
                        seen_changed = 0, labels = ?12
                     WHERE project_id = ?1 AND ext_id = ?2",
                    params![
                        project_id,
                        f.ext_id,
                        f.title,
                        f.url,
                        f.status_label,
                        f.category_label,
                        f.version,
                        f.author,
                        f.body,
                        f.comment_count,
                        f.changed_at,
                        labels
                    ],
                )?;
                delta.changed_issues += 1;
            }
            // Unchanged issue, but the stored body / comment count / label set may
            // predate a rendering format change (e.g. plain text before raw HTML)
            // or a label edit that left updated_at alone.
            Some(_) => {
                tx.execute(
                    "UPDATE issues SET body = ?3, comment_count = ?4, labels = ?5
                     WHERE project_id = ?1 AND ext_id = ?2
                       AND (body != ?3 OR comment_count != ?4 OR labels != ?5)",
                    params![project_id, f.ext_id, f.body, f.comment_count, labels],
                )?;
            }
        }
    }
    tx.execute(
        "UPDATE projects SET last_refreshed = ?2, last_error = NULL WHERE id = ?1",
        params![project_id, now],
    )?;
    tx.commit()?;
    // A first fetch is a backlog import: the rows are marked seen above, so the
    // deltas must not claim otherwise (see the `initial_sync` note in the docs).
    if initial_sync {
        return Ok(SyncDelta::default());
    }
    delta.target_issue_id = target.map(|(id, _, _)| id);
    Ok(delta)
}

/// Store the per-project notification switches.
pub fn set_notifications(
    conn: &Connection,
    project_id: i64,
    notify_new_issues: bool,
    notify_new_comments: bool,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE projects SET notify_new_issues = ?2, notify_new_comments = ?3 WHERE id = ?1",
        params![
            project_id,
            notify_new_issues as i64,
            notify_new_comments as i64
        ],
    )?;
    Ok(())
}

pub fn record_project_error(
    conn: &Connection,
    project_id: i64,
    error: &str,
) -> Result<(), rusqlite::Error> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE projects SET last_error = ?2, last_refreshed = ?3 WHERE id = ?1",
        params![project_id, error, now],
    )?;
    Ok(())
}

pub struct IssueRow {
    pub id: i64,
    pub project_id: i64,
    pub project_name: String,
    pub source: String,
    pub ext_id: String,
    pub title: String,
    pub url: String,
    pub status_label: String,
    pub category_label: String,
    pub version: String,
    pub author: String,
    pub body: String,
    pub comment_count: i64,
    pub labels: Vec<String>,
    pub favorite: bool,
    pub created_at: i64,
    pub changed_at: i64,
    pub seen: bool,
    pub seen_changed: bool,
}

/// project_id = 0 lists issues of every project.
pub fn list_issues(
    conn: &Connection,
    project_id: i64,
    unseen_only: bool,
    favorites_only: bool,
) -> Result<Vec<IssueRow>, rusqlite::Error> {
    let mut sql = String::from(
        "SELECT i.id, i.project_id, p.name, p.kind, i.ext_id, i.title, i.url, i.status_label,
                i.category_label, i.version, i.author, i.body, i.comment_count, i.favorite, i.created_at, i.changed_at, i.seen, i.seen_changed, i.labels
         FROM issues i JOIN projects p ON p.id = i.project_id",
    );
    let mut clauses: Vec<&str> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if project_id > 0 {
        clauses.push("i.project_id = ?");
        params_vec.push(Box::new(project_id));
    }
    if favorites_only {
        clauses.push("i.favorite = 1");
    }
    if unseen_only {
        clauses.push("(i.seen = 0 OR i.seen_changed = 0)");
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY i.changed_at DESC LIMIT 1000");

    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(params_ref.as_slice(), |r| {
            Ok(IssueRow {
                id: r.get(0)?,
                project_id: r.get(1)?,
                project_name: r.get(2)?,
                source: r.get(3)?,
                ext_id: r.get(4)?,
                title: r.get(5)?,
                url: r.get(6)?,
                status_label: r.get(7)?,
                category_label: r.get(8)?,
                version: r.get(9)?,
                author: r.get(10)?,
                body: r.get(11)?,
                comment_count: r.get(12)?,
                favorite: r.get::<_, i64>(13)? != 0,
                created_at: r.get(14)?,
                changed_at: r.get(15)?,
                seen: r.get::<_, i64>(16)? != 0,
                seen_changed: r.get::<_, i64>(17)? != 0,
                labels: split_labels(&r.get::<_, String>(18)?),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_issue(conn: &Connection, issue_id: i64) -> Result<Option<IssueRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT i.id, i.project_id, p.name, p.kind, i.ext_id, i.title, i.url, i.status_label,
                i.category_label, i.version, i.author, i.body, i.comment_count, i.favorite, i.created_at, i.changed_at, i.seen, i.seen_changed, i.labels
         FROM issues i JOIN projects p ON p.id = i.project_id WHERE i.id = ?1",
    )?;
    let row = stmt
        .query_row([issue_id], |r| {
            Ok(IssueRow {
                id: r.get(0)?,
                project_id: r.get(1)?,
                project_name: r.get(2)?,
                source: r.get(3)?,
                ext_id: r.get(4)?,
                title: r.get(5)?,
                url: r.get(6)?,
                status_label: r.get(7)?,
                category_label: r.get(8)?,
                version: r.get(9)?,
                author: r.get(10)?,
                body: r.get(11)?,
                comment_count: r.get(12)?,
                favorite: r.get::<_, i64>(13)? != 0,
                created_at: r.get(14)?,
                changed_at: r.get(15)?,
                seen: r.get::<_, i64>(16)? != 0,
                seen_changed: r.get::<_, i64>(17)? != 0,
                labels: split_labels(&r.get::<_, String>(18)?),
            })
        })
        .optional()?;
    Ok(row)
}

pub fn mark_issue_seen(conn: &Connection, issue_id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE issues SET seen = 1, seen_changed = 1 WHERE id = ?1",
        [issue_id],
    )?;
    Ok(())
}

pub fn mark_project_seen(conn: &Connection, project_id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE issues SET seen = 1, seen_changed = 1 WHERE project_id = ?1",
        [project_id],
    )?;
    Ok(())
}

pub fn mark_all_seen(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute("UPDATE issues SET seen = 1, seen_changed = 1", [])?;
    Ok(())
}

/// Flip an issue's favorite flag; returns the new state.
pub fn toggle_favorite(conn: &Connection, issue_id: i64) -> Result<bool, rusqlite::Error> {
    conn.execute(
        "UPDATE issues SET favorite = 1 - favorite WHERE id = ?1",
        [issue_id],
    )?;
    let fav: i64 = conn.query_row(
        "SELECT favorite FROM issues WHERE id = ?1",
        [issue_id],
        |r| r.get(0),
    )?;
    Ok(fav != 0)
}

pub fn favorites_count(conn: &Connection) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM issues WHERE favorite = 1",
        [],
        |r| r.get(0),
    )
}

/// Persist how many issue pages have been fetched; only ever grows.
pub fn set_pages_fetched(conn: &Connection, project_id: i64, pages: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE projects SET pages_fetched = MAX(pages_fetched, ?2) WHERE id = ?1",
        params![project_id, pages],
    )?;
    Ok(())
}

pub fn get_pages_fetched(conn: &Connection, project_id: i64) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT pages_fetched FROM projects WHERE id = ?1",
        [project_id],
        |r| r.get(0),
    )
}

pub fn rename_project(conn: &Connection, project_id: i64, name: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE projects SET name = ?2 WHERE id = ?1",
        params![project_id, name],
    )?;
    Ok(())
}

/// Persist a new display order; `ids` must contain every project id.
pub fn reorder_projects(conn: &mut Connection, ids: &[i64]) -> Result<(), rusqlite::Error> {
    let tx = conn.transaction()?;
    for (pos, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE projects SET sort_order = ?2 WHERE id = ?1",
            params![id, (pos + 1) as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn delete_issue(
    conn: &Connection,
    project_id: i64,
    ext_id: &str,
) -> Result<(), rusqlite::Error> {
    // Comments first: the subquery needs the issue row still present, and
    // without the foreign_keys pragma the note rows would leak otherwise.
    // Same shape as prune_missing_issues, scoped to one issue.
    conn.execute(
        "DELETE FROM comments WHERE issue_id IN (SELECT id FROM issues WHERE project_id = ?1 AND ext_id = ?2)",
        params![project_id, ext_id],
    )?;
    conn.execute(
        "DELETE FROM issues WHERE project_id = ?1 AND ext_id = ?2",
        params![project_id, ext_id],
    )?;
    Ok(())
}

pub fn delete_project(conn: &Connection, project_id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM comments WHERE issue_id IN (SELECT id FROM issues WHERE project_id = ?1)",
        [project_id],
    )?;
    conn.execute("DELETE FROM issues WHERE project_id = ?1", [project_id])?;
    conn.execute("DELETE FROM projects WHERE id = ?1", [project_id])?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [key],
        |r| r.get(0),
    )
    .optional()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Insert newly fetched comments (unseen) and return the full cached thread
/// for an issue, oldest first, with `is_new` flags. Existing comments keep
/// their seen state and get their body refreshed in place.
/// GitLab serves public notes with `author` stripped, so an unauthenticated
/// read returns empty names - never overwrite a known author with an empty
/// one. Body, created_at and system do get refreshed on every open.
pub fn upsert_comments(
    conn: &mut Connection,
    issue_id: i64,
    fetched: &[crate::models::FetchedComment],
    preserve_authors: bool,
) -> Result<Vec<crate::models::Comment>, rusqlite::Error> {
    let tx = conn.transaction()?;
    for c in fetched {
        let author_sql = if preserve_authors {
            // COALESCE keeps the stored value whenever the incoming one is
            // empty; a real username always wins over an empty string.
            "COALESCE(NULLIF(excluded.author, ''), author)"
        } else {
            "excluded.author"
        };
        tx.execute(
            &format!(
                "INSERT INTO comments (issue_id, ext_id, author, body, created_at, system, seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)
                 ON CONFLICT(issue_id, ext_id) DO UPDATE SET
                    author = {author_sql},
                    body = excluded.body,
                    created_at = excluded.created_at,
                    system = excluded.system"
            ),
            params![issue_id, c.ext_id, c.author, c.body, c.created_at, c.system as i64],
        )?;
    }
    tx.commit()?;

    let mut stmt = conn.prepare(
        "SELECT ext_id, author, body, created_at, system, seen
         FROM comments WHERE issue_id = ?1 ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt
        .query_map([issue_id], |r| {
            Ok(crate::models::Comment {
                ext_id: r.get(0)?,
                author: r.get(1)?,
                body: r.get(2)?,
                created_at: r.get(3)?,
                system: r.get::<_, i64>(4)? != 0,
                is_new: r.get::<_, i64>(5)? == 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn mark_comments_seen(conn: &Connection, issue_id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE comments SET seen = 1 WHERE issue_id = ?1 AND seen = 0",
        [issue_id],
    )?;
    Ok(())
}

/// Remove a single cached comment, used after its GitLab note has been
/// deleted. Covers issues whose note list is too long to fetch in one page,
/// where `prune_comments` must not run.
pub fn delete_comment(
    conn: &Connection,
    issue_id: i64,
    ext_id: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM comments WHERE issue_id = ?1 AND ext_id = ?2",
        params![issue_id, ext_id],
    )?;
    Ok(())
}

/// Drop the cached comments of an issue whose ext_id is not in `keep_ext_ids`.
/// Only safe after a fetch that returned the *complete* thread: GitLab notes
/// come in one page of 100, so a shorter response is everything there is, and
/// notes deleted upstream (in this app or in the web UI) would otherwise stay
/// in the cache forever.
pub fn prune_comments(
    conn: &Connection,
    issue_id: i64,
    keep_ext_ids: &[String],
) -> Result<(), rusqlite::Error> {
    for ext_id in cached_comment_ids(conn, issue_id)?
        .iter()
        .filter(|id| !keep_ext_ids.contains(id))
    {
        conn.execute(
            "DELETE FROM comments WHERE issue_id = ?1 AND ext_id = ?2",
            params![issue_id, ext_id],
        )?;
    }
    Ok(())
}

/// Drop cache rows for work items that no longer exist on GitLab. `keep` is
/// the project's full issue id set as fetched upstream, so the cached rows
/// that are not in it are deleted along with their comments. The comment
/// rows have to go first: there is no foreign-key enforcement, so a pruned
/// issue row would otherwise leave its comments as orphans.
///
/// Returns the number of issue rows removed.
pub fn prune_missing_issues(
    conn: &Connection,
    project_id: i64,
    keep: &[String],
) -> Result<i64, rusqlite::Error> {
    let cached: Vec<String> = {
        let mut stmt = conn.prepare("SELECT ext_id FROM issues WHERE project_id = ?1")?;
        let rows = stmt.query_map(params![project_id], |r| r.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let keep_set: std::collections::HashSet<&str> = keep.iter().map(String::as_str).collect();
    // Comments first: the subquery needs the issue row to still be present,
    // and without it a pruned issue would leak its cached notes.
    let mut stmt_comment = conn.prepare("DELETE FROM comments WHERE issue_id IN (SELECT id FROM issues WHERE project_id = ?1 AND ext_id = ?2)")?;
    let mut stmt_issue = conn.prepare("DELETE FROM issues WHERE project_id = ?1 AND ext_id = ?2")?;
    let mut removed = 0i64;
    for ext in &cached {
        if keep_set.contains(ext.as_str()) {
            continue;
        }
        // The comment delete must run before the issue row goes, because the
        // subquery needs the issue id to still be present.
        stmt_comment.execute(params![project_id, ext])?;
        stmt_issue.execute(params![project_id, ext])?;
        removed += 1;
    }
    Ok(removed)
}

/// Labels are stored one per line; GitLab label names cannot contain a newline.
fn join_labels(labels: &[String]) -> String {
    labels.join("\n")
}

fn split_labels(stored: &str) -> Vec<String> {
    stored
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Overwrite the label set of a cached issue, so a label write is reflected
/// immediately instead of only after the next refresh.
pub fn set_issue_labels(
    conn: &Connection,
    issue_id: i64,
    labels: &[String],
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE issues SET labels = ?2 WHERE id = ?1",
        params![issue_id, join_labels(labels)],
    )?;
    Ok(())
}

/// Replace the cached label list of a project. Called after a labels-API read,
/// so re-fetching replaces the set instead of stacking near-duplicates.
pub fn replace_project_labels(
    conn: &mut Connection,
    project_id: i64,
    labels: &[crate::models::Label],
) -> Result<(), rusqlite::Error> {
    let tx = conn.transaction()?;
    let now = chrono::Utc::now().timestamp();
    tx.execute(
        "DELETE FROM project_labels WHERE project_id = ?1",
        params![project_id],
    )?;
    for l in labels {
        tx.execute(
            "INSERT OR IGNORE INTO project_labels (project_id, name, color, description, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_id, l.name, l.color, l.description, now],
        )?;
    }
    tx.commit()
}

/// The cached labels of a project, alphabetical, for the picker and its
/// autocompletion. Empty until the labels have been fetched once.
pub fn list_project_labels(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<crate::models::Label>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT name, color, description FROM project_labels
         WHERE project_id = ?1 ORDER BY name COLLATE NOCASE ASC",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok(crate::models::Label {
                name: r.get(0)?,
                color: r.get(1)?,
                description: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Comment ext_ids already cached for an issue, used to fetch only what's new.
pub fn cached_comment_ids(conn: &Connection, issue_id: i64) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT ext_id FROM comments WHERE issue_id = ?1")?;
    let rows = stmt
        .query_map([issue_id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn comment_bodies(conn: &Connection, issue_id: i64) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT body FROM comments WHERE issue_id = ?1")?;
    let rows = stmt
        .query_map([issue_id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FetchedIssue;

    fn issue(ext_id: &str, changed_at: i64, comment_count: i64) -> FetchedIssue {
        FetchedIssue {
            ext_id: ext_id.into(),
            title: format!("issue {ext_id}"),
            url: format!("https://www.drupal.org/i/{ext_id}"),
            status_label: "Active".into(),
            category_label: "Bug".into(),
            version: String::new(),
            author: "someone".into(),
            created_at: 100,
            changed_at,
            body: "body".into(),
            comment_count,
            labels: Vec::new(),
        }
    }

    /// A connection at the current schema with one drupal project.
    fn seeded() -> (Connection, i64) {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO projects (kind, key, name, url) VALUES ('drupal', '1234', 'p', 'u')",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        (conn, id)
    }

    fn counts(d: &SyncDelta) -> (i64, i64, i64) {
        (d.new_issues, d.changed_issues, d.new_comments)
    }

    fn row_id(conn: &Connection, pid: i64, ext_id: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM issues WHERE project_id = ?1 AND ext_id = ?2",
            params![pid, ext_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn init_schema_can_run_twice() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
    }

    #[test]
    fn prune_missing_issues_removes_rows_not_in_the_keep_set() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 2), issue("2", 11, 0)], true).unwrap();
        // A cached comment on the doomed row must go with it.
        let iid = row_id(&conn, pid, "1");
        conn.execute(
            "INSERT INTO comments (issue_id, ext_id, author, body, created_at) VALUES (?1, 'n1', 'a', 'b', 1)",
            [iid],
        )
        .unwrap();
        // "2" still exists upstream, "1" does not -> exactly one row goes.
        assert_eq!(prune_missing_issues(&conn, pid, &["2".into()]).unwrap(), 1);
        assert!(
            conn.query_row(
                "SELECT COUNT(*) FROM issues WHERE project_id = ?1 AND ext_id = '1'",
                [pid],
                |r| r.get::<_, i64>(0),
            )
            .unwrap() == 0,
            "the deleted work item is gone from the cache"
        );
        assert!(
            conn.query_row(
                "SELECT COUNT(*) FROM comments WHERE issue_id = ?1",
                [iid],
                |r| r.get::<_, i64>(0),
            )
            .unwrap() == 0,
            "the cached comments of a pruned issue are removed with it"
        );
        assert!(
            conn.query_row(
                "SELECT COUNT(*) FROM issues WHERE project_id = ?1 AND ext_id = '2'",
                [pid],
                |r| r.get::<_, i64>(0),
            )
            .unwrap() == 1,
            "kept issues survive the prune"
        );
    }

    #[test]
    fn prune_missing_issues_with_empty_keep_set_wipes_the_project() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 0), issue("2", 11, 0)], true).unwrap();
        assert_eq!(prune_missing_issues(&conn, pid, &[]).unwrap(), 2);
        assert_eq!(row_id_result(&conn, pid, "1"), None);
    }

    #[test]
    fn delete_issue_removes_the_row_and_its_comments() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 2), issue("2", 11, 0)], true).unwrap();
        let iid = row_id(&conn, pid, "1");
        conn.execute(
            "INSERT INTO comments (issue_id, ext_id, author, body, created_at) VALUES (?1, 'n1', 'a', 'b', 1)",
            [iid],
        )
        .unwrap();
        delete_issue(&conn, pid, "1").unwrap();
        assert!(
            conn.query_row(
                "SELECT COUNT(*) FROM issues WHERE project_id = ?1 AND ext_id = '1'",
                [pid],
                |r| r.get::<_, i64>(0),
            )
            .unwrap() == 0,
            "the deleted row is gone"
        );
        assert!(
            conn.query_row(
                "SELECT COUNT(*) FROM comments WHERE issue_id = ?1",
                [iid],
                |r| r.get::<_, i64>(0),
            )
            .unwrap() == 0,
            "the comments of a deleted issue are removed with it"
        );
        assert!(
            conn.query_row(
                "SELECT COUNT(*) FROM issues WHERE project_id = ?1 AND ext_id = '2'",
                [pid],
                |r| r.get::<_, i64>(0),
            )
            .unwrap() == 1,
            "the sibling row survives"
        );
    }

    fn row_id_result(conn: &Connection, pid: i64, ext: &str) -> Option<i64> {
        conn.query_row(
            "SELECT id FROM issues WHERE project_id = ?1 AND ext_id = ?2",
            params![pid, ext],
            |r| r.get(0),
        )
        .ok()
    }

    #[test]
    fn first_fetch_imports_the_backlog_without_deltas() {
        let (mut conn, pid) = seeded();
        let d = upsert_issues(&mut conn, pid, &[issue("1", 10, 4), issue("2", 11, 0)], true).unwrap();
        assert_eq!(counts(&d), (0, 0, 0), "backlog is not news");
        assert_eq!(d.target_issue_id, None);
        let unseen: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM issues WHERE project_id = ?1 AND seen = 0",
                [pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unseen, 0, "backlog rows are marked seen");
    }

    #[test]
    fn a_new_issue_counts_as_new_not_as_new_comments() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 4)], true).unwrap();
        let d = upsert_issues(&mut conn, pid, &[issue("1", 10, 4), issue("2", 12, 7)], true).unwrap();
        assert_eq!(counts(&d), (1, 0, 0));
        assert_eq!(d.target_issue_id, Some(row_id(&conn, pid, "2")));
    }

    #[test]
    fn comment_increase_on_a_known_issue_is_counted() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 4)], true).unwrap();
        let d = upsert_issues(&mut conn, pid, &[issue("1", 20, 9)], true).unwrap();
        assert_eq!(counts(&d), (0, 1, 5));
        assert_eq!(d.target_issue_id, Some(row_id(&conn, pid, "1")));
    }

    #[test]
    fn comment_increase_counts_even_when_the_host_keeps_changed_at() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 4)], true).unwrap();
        let d = upsert_issues(&mut conn, pid, &[issue("1", 10, 6)], true).unwrap();
        assert_eq!(counts(&d), (0, 0, 2));
        assert_eq!(d.target_issue_id, Some(row_id(&conn, pid, "1")));
    }

    #[test]
    fn comment_decrease_is_never_reported() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 9)], true).unwrap();
        let d = upsert_issues(&mut conn, pid, &[issue("1", 20, 2)], true).unwrap();
        assert_eq!(counts(&d).2, 0);
        assert_eq!(d.target_issue_id, None, "a status change alone is not news");
    }

    #[test]
    fn deltas_accumulate_across_the_issues_in_one_page() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 1), issue("2", 11, 1)], true).unwrap();
        let d = upsert_issues(&mut conn, pid, &[issue("1", 20, 3), issue("2", 21, 4)], true).unwrap();
        assert_eq!(counts(&d), (0, 2, 5));
        assert_eq!(d.target_issue_id, Some(row_id(&conn, pid, "2")), "newest wins");
    }

    #[test]
    fn a_new_issue_outranks_an_older_comment_when_the_target_is_chosen() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("old", 10, 1)], true).unwrap();
        // "old" gains a comment at the same timestamp a new issue reports.
        let d = upsert_issues(
            &mut conn,
            pid,
            &[issue("new", 10, 0), issue("old", 10, 4)],
            true,
        )
        .unwrap();
        assert_eq!(counts(&d), (1, 0, 3));
        assert_eq!(d.target_issue_id, Some(row_id(&conn, pid, "new")));
    }

    #[test]
    fn nothing_actionable_leaves_no_target() {
        let (mut conn, pid) = seeded();
        upsert_issues(&mut conn, pid, &[issue("1", 10, 4)], true).unwrap();
        let d = upsert_issues(&mut conn, pid, &[issue("1", 10, 4)], true).unwrap();
        assert_eq!(d, SyncDelta::default());
    }

    /// The real upgrade path: a database written by the previous release gains
    /// both switches on `ALTER TABLE`, existing projects default to on, and no
    /// row is lost.
    #[test]
    fn a_legacy_database_is_upgraded_with_notifications_on() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL, key TEXT NOT NULL, name TEXT NOT NULL,
                url TEXT NOT NULL, last_refreshed INTEGER, last_error TEXT,
                sort_order INTEGER NOT NULL DEFAULT 0,
                pages_fetched INTEGER NOT NULL DEFAULT 0,
                UNIQUE(kind, key));
             CREATE TABLE issues (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL, ext_id TEXT NOT NULL,
                title TEXT NOT NULL, url TEXT NOT NULL DEFAULT '',
                status_label TEXT NOT NULL DEFAULT '',
                category_label TEXT NOT NULL DEFAULT '',
                version TEXT NOT NULL DEFAULT '', author TEXT NOT NULL DEFAULT '',
                body TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL DEFAULT 0,
                changed_at INTEGER NOT NULL DEFAULT 0, first_seen INTEGER NOT NULL DEFAULT 0,
                seen INTEGER NOT NULL DEFAULT 1, seen_changed INTEGER NOT NULL DEFAULT 1,
                comment_count INTEGER NOT NULL DEFAULT 0, favorite INTEGER NOT NULL DEFAULT 0,
                UNIQUE(project_id, ext_id));
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO projects (kind, key, name, url, sort_order)
                 VALUES ('drupal', '5555', 'legacy', 'u', 1);",
        )
        .unwrap();

        init_schema(&conn).unwrap();

        let row = get_project(&conn, 1).unwrap().expect("legacy row survives");
        assert_eq!(row.name, "legacy");
        assert!(
            row.notify_new_issues && row.notify_new_comments,
            "existing projects default to notifying on both signals"
        );
        assert_eq!(get_project(&conn, 1).unwrap().unwrap().issue_count, 0);

        // Every issue read selects the label column, so a database from before
        // it existed is only usable once the ALTER has run.
        let mut f = issue("77", 10, 0);
        f.labels = vec!["category::bug".into()];
        upsert_issues(&mut conn, 1, &[f], true).unwrap();
        let iid = row_id(&conn, 1, "77");
        assert_eq!(
            get_issue(&conn, iid).unwrap().unwrap().labels,
            vec!["category::bug".to_string()]
        );
        assert_eq!(list_issues(&conn, 1, false, false).unwrap()[0].labels.len(), 1);
    }

    fn comment(ext_id: &str, system: bool) -> crate::models::FetchedComment {
        crate::models::FetchedComment {
            ext_id: ext_id.into(),
            author: "steffen".into(),
            body: format!("comment {ext_id}"),
            created_at: 100,
            system,
        }
    }

    /// An issue row with three cached comments (one of them a system note).
    fn thread(conn: &mut Connection, pid: i64, issue_ext: &str) -> i64 {
        upsert_issues(conn, pid, &[issue(issue_ext, 10, 3)], true).unwrap();
        let iid = row_id(conn, pid, issue_ext);
        upsert_comments(
            conn,
            iid,
            &[
                comment("a", false),
                comment("b", false),
                comment("c", true),
            ],
            false,
        )
        .unwrap();
        iid
    }

    /// An unauthenticated/empty-author re-fetch must not clobber a known
    /// author name, but must still refresh body and system flags.
    #[test]
    fn an_empty_author_keeps_the_stored_name() {
        let (mut conn, pid) = seeded();
        let iid = thread(&mut conn, pid, "1");
        // Seed a known author on comment "a"
        conn.execute(
            "UPDATE comments SET author = 'alice' WHERE issue_id = ?1 AND ext_id = 'a'",
            [iid],
        )
        .unwrap();
        // Simulate a GitLab re-fetch that returns an empty author for "a"
        // but a fresh body, and a real author for "b"
        let refetched: Vec<crate::models::FetchedComment> = vec![
            crate::models::FetchedComment {
                ext_id: "a".into(),
                author: String::new(),
                body: "edited body".into(),
                created_at: 1,
                system: false,
            },
            crate::models::FetchedComment {
                ext_id: "b".into(),
                author: "bob".into(),
                body: "new body".into(),
                created_at: 2,
                system: false,
            },
        ];
        let rows = upsert_comments(&mut conn, iid, &refetched, true).unwrap();
        let by_ext: std::collections::HashMap<String, _> =
            rows.iter().map(|r| (r.ext_id.clone(), r.author.clone())).collect();
        assert_eq!(by_ext.get("a").unwrap(), "alice", "empty author must not clobber a known name");
        assert_eq!(by_ext.get("b").unwrap(), "bob", "a real author name must land");
        // body did refresh
        let body: String = conn
            .query_row(
                "SELECT body FROM comments WHERE issue_id = ?1 AND ext_id = 'a'",
                [iid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body, "edited body");
    }

    #[test]
    fn deleting_a_note_removes_only_that_cached_comment() {
        let (mut conn, pid) = seeded();
        let iid = thread(&mut conn, pid, "1");

        delete_comment(&conn, iid, "b").unwrap();

        assert_eq!(
            cached_comment_ids(&conn, iid).unwrap(),
            vec!["a".to_string(), "c".to_string()],
            "the edited-away note goes, the rest of the thread stays"
        );
    }

    #[test]
    fn pruning_after_a_complete_fetch_drops_server_side_deletions() {
        let (mut conn, pid) = seeded();
        let iid = thread(&mut conn, pid, "1");
        let other = thread(&mut conn, pid, "2");

        // The next poll of issue 1 no longer returns "b" — someone deleted it.
        prune_comments(&conn, iid, &["a".to_string(), "c".to_string()]).unwrap();

        assert_eq!(
            cached_comment_ids(&conn, iid).unwrap(),
            vec!["a".to_string(), "c".to_string()]
        );
        assert_eq!(
            cached_comment_ids(&conn, other).unwrap(),
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "pruning one issue never touches another's thread"
        );
    }

    #[test]
    fn labels_survive_the_issue_cache_and_can_be_written_back() {
        let (mut conn, pid) = seeded();
        let mut f = issue("1", 10, 0);
        f.labels = vec!["category::bug".into(), "priority::major".into()];
        upsert_issues(&mut conn, pid, &[f], true).unwrap();

        let iid = row_id(&conn, pid, "1");
        assert_eq!(
            get_issue(&conn, iid).unwrap().unwrap().labels,
            vec!["category::bug".to_string(), "priority::major".to_string()]
        );
        assert_eq!(list_issues(&conn, pid, false, false).unwrap()[0].labels.len(), 2);

        // A local write is visible without waiting for the next refresh.
        set_issue_labels(&conn, iid, &["category::task".into(), "needs: review".into()]).unwrap();
        assert_eq!(
            get_issue(&conn, iid).unwrap().unwrap().labels,
            vec!["category::task".to_string(), "needs: review".to_string()]
        );
    }

    #[test]
    fn a_label_change_alone_still_refreshes_an_unchanged_issue() {
        let (mut conn, pid) = seeded();
        let mut f = issue("1", 10, 0);
        f.labels = vec!["state::needsReview".into()];
        upsert_issues(&mut conn, pid, &[f.clone()], true).unwrap();
        let iid = row_id(&conn, pid, "1");
        // Same changed_at, different labels: the row must not keep the old set.
        f.labels = vec!["state::rtbc".into()];
        upsert_issues(&mut conn, pid, &[f], true).unwrap();
        assert_eq!(
            get_issue(&conn, iid).unwrap().unwrap().labels,
            vec!["state::rtbc".to_string()]
        );
    }

    #[test]
    fn project_label_caches_are_replaced_per_project() {
        let (mut conn, pid) = seeded();
        let other = {
            conn.execute(
                "INSERT INTO projects (kind, key, name, url) VALUES ('gitlab', 'g/p2', 'q', 'u')",
                [],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let label = |name: &str| crate::models::Label {
            name: name.into(),
            color: "#d9534f".into(),
            description: String::new(),
        };

        replace_project_labels(&mut conn, pid, &[label("category::bug"), label("tags::dxiw")]).unwrap();
        replace_project_labels(&mut conn, other, &[label("component::admin")]).unwrap();
        // Re-fetching replaces rather than stacks, and a re-added name is not
        // duplicated.
        replace_project_labels(&mut conn, pid, &[label("category::bug"), label("priority::normal")]).unwrap();

        assert_eq!(
            list_project_labels(&conn, pid)
                .unwrap()
                .into_iter()
                .map(|l| l.name)
                .collect::<Vec<_>>(),
            vec!["category::bug".to_string(), "priority::normal".to_string()]
        );
        assert_eq!(list_project_labels(&conn, other).unwrap().len(), 1);
    }

    #[test]
    fn notification_switches_default_on_and_round_trip() {
        let (conn, pid) = seeded();
        let row = get_project(&conn, pid).unwrap().unwrap();
        assert!(row.notify_new_issues && row.notify_new_comments);

        set_notifications(&conn, pid, false, true).unwrap();
        let row = get_project(&conn, pid).unwrap().unwrap();
        assert!(!row.notify_new_issues);
        assert!(row.notify_new_comments);

        let listed = list_projects(&conn).unwrap();
        assert_eq!(listed[0].notify_new_issues, false);
        assert_eq!(listed[0].notify_new_comments, true);
    }
}
