# AGENTS.md — guidance for coding agents

Drupal Issue Hub — a Tauri 2 desktop app (React + TypeScript frontend,
Rust backend) that tracks issue queues of Drupal.org projects and
git.drupalcode.org work items, with optional write support for GitLab issues.

## Commands

```sh
npm install                # install frontend deps (Node 22+)
npm run tauri dev          # dev mode: vite + debug binary, hot reload
npm run tauri build        # release bundle (macOS .app/.dmg here)
npm run build              # typecheck (tsc) + vite build — run before commits
cargo check --manifest-path src-tauri/Cargo.toml --message-format short
cargo test --release --lib --manifest-path src-tauri/Cargo.toml
node scripts/sync-version.mjs <x.y.z>   # sync version into Cargo.toml/lock + tauri.conf.json
npm run tauri icon src/assets/logo.svg  # regenerate icon set (source must be square)
```

There is no vitest/jest; frontend correctness is guarded by `tsc` in
`npm run build`, backend units live as `#[cfg(test)]` in Rust modules
(e.g. asset extraction in `drupal.rs`, notification deltas and the
notification switches in `db.rs`). `db::init_schema` exists so those tests can
bring an in-memory connection up to the current schema — keep it callable twice
and keep `open()` a thin wrapper over it.

## Layout & responsibilities

```
src/                     frontend (React 19 + TS)
  api.ts                 ALL backend access goes through here: one typed
                         invoke wrapper per Tauri command
  render.ts              renderMarkup(source, body): the ONLY place that turns
                         untrusted markup into HTML (marked for GitLab
                         markdown, DOMPurify always)
  App.tsx                state orchestration, auto-poll timer, notifications
  components/            Sidebar, IssueTable, IssueDetail, AddProjectModal,
                         SettingsModal, LabelEditor
src-tauri/src/
  lib.rs                 builder setup, client construction, command registry
  commands.rs            all #[tauri::command]s; owns AppState{db, http}
  db.rs                  SQLite schema, migrations, every SQL statement
  drupal.rs              drupal.org api-d7 client + issue-page scraping
  gitlab.rs              GitLab REST client (issues, notes, edits, token)
  net.rs                 send_retry(): request with one transport-failure retry
```

## Hard rules (do not violate)

0. **Never use `window.confirm` / `alert` / `prompt`.** Tauri's WKWebView does
   not implement them on macOS — they silently return `false`/`undefined`
   (deletion of projects was broken this way). Use an in-app modal
   (see the delete-project modal in `App.tsx`) or a plugin.
1. **Never hold the DB mutex across an `await`.** `AppState.db` is a
   `std::sync::Mutex`; take it in a short block, clone/extract what you need,
   drop it, then do network work. See `refresh_one` for the pattern.
2. **All commands return `Result<T, String>`** and rusqlite errors are mapped
   through the `se()` helper. Network errors are formatted with
   `drupal::err_chain()` so the full cause chain is visible (reqwest's Display
   alone hides the real error).
3. **Host allowlist**: outgoing requests only go to `www.drupal.org`,
   `drupal.org`, `git.drupalcode.org` over https, and each fetch site
   validates its own URL (SSRF guard in `drupal.rs::fetch_issue_assets` —
   keep that pattern for any new fetch of stored/remote URLs). Never add
   fetches of user/DB-supplied URLs without the same check.
4. **Untrusted markup is always sanitized** through `renderMarkup` (DOMPurify).
   Never `dangerouslySetInnerHTML` anything that did not come from it.
5. **DB migrations**: add columns/tables via the `pragma_table_info` check +
   `ALTER TABLE` pattern in `db.rs::open` (see `comment_count`, `favorite`,
   `sort_order`). Never edit existing migration blocks.
6. **Tauri commands must be registered** in `generate_handler!` in `lib.rs`
   and given a typed wrapper in `src/api.ts`.

## API quirks (things that look like bugs but aren't)

- drupal.org issue list uses `limit=100` (NOT `pagesize` — the server rejects
  it with a 412 text/plain error that used to surface as "invalid JSON").
- drupal.org issue bodies in the list response are filtered HTML — stored raw,
  rendered via DOMPurify. Do not strip tags in Rust anymore.
- drupal.org file entity API is closed (403). Patch files are scraped from the
  rendered issue page (`/files/issues/*.patch|.diff`, mime `x-diff`).
- drupal.org comments have no `nid` filter on `comment.json`; the issue node
  lists comment ids (`comments[].id`) and each comment is fetched by `cid`.
  Fetch incrementally (skip cached ids, cap 60 per open, chunks of 8) plus the
  newest 10 cached ones always — comment edits must be picked up while
  viewing (`fetch_issue_node` also returns the issue's current fields, which
  get upserted so body/status edits surface with the UPD badge).
- GitLab comment writes are **author-only**: `edit_issue_comment` /
  `delete_issue_comment` hit `PUT`/`DELETE …/notes/:id`, which GitLab allows just
  for the note's author, so `IssueDetail` shows ✎/🗑 only on non-system comments
  whose `author` matches `settings.gitlab_user`. Deletions are kept in sync two
  ways: the cached row is dropped right after a successful delete, and
  `get_comments` prunes notes that are gone upstream — but **only when
  `gitlab::fetch_comments` reported the thread as complete** (it pages through
  notes and returns `(notes, complete)`; a short page ends the list, the 10-page
  cap does not). Pruning a partial read would erase real history. Never prune on
  the drupal.org path; its comment fetch is a deliberate subset.
- **Labels are GitLab-only** and written **differentially**: `set_issue_labels`
  sends `add_labels`/`remove_labels`, not `labels` — the latter replaces the
  whole set and would wipe `component::*`, `tags::*` and a project's own labels,
  which `fetch_issues` deliberately keeps verbatim in `issues.labels`. GitLab's
  scoped labels separate key and value with a **double colon** (`state::needsWork`,
  `category::bug`) — splitting on one colon leaves a `:needsWork` value, which is
  the bug shape to avoid. Three consequences worth knowing: `add_labels`
  **creates** a missing label in the project (for everyone — hence the warning in
  `LabelEditor`), labeling needs the **Planner role or above** so a 403 is
  expected on projects you do not triage (drupal.org's fallback is a
  `/do:label <name>` comment), and names containing a comma or newline are refused
  before the request because the API takes a comma-separated string while the
  cache stores one label per line. GitLab keeps two `key::value` labels of one key
  off an issue only on Premium and above, so the **picker enforces the one
  value per namespace** itself. `read_labels` understands both spellings —
  migrated `"Status: Needs review"` (single colon) and scoped
  `"state::needsReview"` — and humanises values back to drupal.org wording so
  `statusClass()` and the column filters keep matching. `LabelEditor` renders in
  the issue's **meta block** (label pills, then a `＋ label` trigger that expands
  the picker in place) — not in the "Your tools" area, where it sat below the whole
  discussion and went unnoticed. Labels already carried by the status/category/
  version pills are not repeated there, and pills beyond six collapse behind
  `+N more`.
- A project's label list is fetched with `get_project_labels` (the labels API is
  401 without a token too) and cached in `project_labels` to drive the picker and
  its autocompletion; the cache is replaced per project on each read, so reusing
  a name never stacks a near-duplicate.
- `settings.hide_system_comments` hides the hosts' "Status changed to …" notes in
  the thread. Display-only: the cache keeps every note and notification counts
  are untouched — on drupal.org those notes are exactly what inflates
  `new_comments` (see above), so hiding them changes no signal.
- git.drupalcode.org notes API returns 401 without a token, even for public
  projects. Related-MR endpoints don't exist (404); MRs are regex-extracted
  from description + cached comments.
- GitLab descriptions of d.o-imported issues are mixed HTML + markdown. A
  block-level closing tag directly followed by markdown (no blank line) would
  be swallowed by marked's HTML-block rule — `terminateHtmlBlocks()` in
  `render.ts` inserts the blank line. Keep that fix when touching rendering.
- Both hosts sit on the same Fastly edge that closes idle keep-alive
  connections; the shared client uses `pool_idle_timeout(15s)`,
  `connect_timeout(10s)`, `timeout(30s)` and `net::send_retry` for one retry.
  Long-lived-process "error sending request" after sleep/wake was diagnosed
  this way — do not remove these knobs.
- drupal.org status/category are numeric codes mapped in `drupal.rs`
  (1=Active, 8=Needs review, 13=Needs work, 14=RTBC, 2=Fixed, …);
  category 1=Bug, 2=Task, 3=Feature, 4=Support, 5=Plan.
- **New comments need no extra requests.** Both queues return a comment count
  in the issue LIST payload (`comment_count` on d.o `node.json`,
  `user_notes_count` on GitLab) and sort by recency of change (`sort=changed`
  / `order_by=updated_at`), so an issue that gained a comment is always inside
  page 0 of a poll. `db::upsert_issues` derives `new_comments` as
  `sum(max(0, incoming - stored))` over issues already in the DB — a decrease
  never counts, a new issue's own comments never count, and the first fetch of
  a project returns `(0, 0, 0)` because a backlog import is not news.
  Consequence: a comment on an issue older than the 100 newest-changed is not
  seen. drupal.org `comment_count` also includes system notes ("Status changed
  to …"), so those inflate `new_comments`; filtering them would need the
  `comments.system` flag, i.e. one fetch per changed issue — deliberately not
  done. Never add comment fetching to the poll path.

## Data model (SQLite, app-data dir)

`projects(id, kind drupal|gitlab, key nid|gitlab-path, name, url,
last_refreshed, last_error, sort_order, notify_new_issues, notify_new_comments)`
— sidebar order is `sort_order` (manual, drag-and-drop), NOT alphabetical.
`issues(project_id, ext_id, …, comment_count, favorite, seen, seen_changed,
first_seen, labels)` — `labels` holds the GitLab label set, one name per line
(empty on drupal.org rows, which have none). `seen=0` ⇒ NEW badge,
`seen_changed=0` ⇒ UPD badge; first fetch
of a project marks everything seen. `favorite` survives refreshes by design.
`comments(issue_id, ext_id, author, body, created_at, system, seen)` — lazy
fetch on issue open, not during polls.
`settings(key, value)` — `poll_enabled`, `poll_interval`, `gitlab_user`,
`hide_system_comments`. New settings are plain key/value rows and need no
migration.
The GitLab token itself is in the macOS Keychain (keyring service
`drupal-issue-hub`, account `gitlab-pat`), never in the DB.
`project_labels(project_id, name, color, description, fetched_at)` — one cached
label list per GitLab project, for the label picker and its autocompletion.

## Notifications

Per-project switches (`notify_new_issues`, `notify_new_comments`, both default
on) are edited from the bell popover in the sidebar row (`Sidebar.tsx`) via
`set_project_notifications`. Rust reports raw deltas plus the two switches on
`RefreshResult`; **`App.tsx` owns the presentation policy** — one
`sendNotification` per project that has something to report, titled with the
project name, body like `2 new issues · 5 new comments`, with permission asked
once per batch. `changed_issues` never notifies. The manual refresh button
passes `notify=false` (you are looking at the window); only the poll timer
passes `true`. `load_more_issues` and the single-issue node fetch ignore the
deltas so lazy loading can never fabricate a notification.

## Source auto-migration

When a drupal.org project's queue is empty, `maybe_migrate_to_gitlab` checks
`git.drupalcode.org/project/<machine-name>` and converts the project row to a
GitLab source if that repo has issues. Repo existence alone is NOT a signal —
every d.o project has a GitLab repo. The machine name is parsed from the
stored `url` (`…/project/issues/<machine>`); keep that URL shape stable.

`add_project` shares the same intent at the entry point: the source is
**decided by where the work items actually live** — if the machine name's
GitLab repo has issues, it is added as a GitLab source even when the
drupal.org endpoint still answers. Only when the GitLab side is empty does
a drupal.org project get stored. This matters because drupal.org's API
keeps serving issues for projects whose queues have moved (menu_block_title
had 25 on d.o and 3 on GitLab simultaneously), and only a GitLab source can
be commented on and edited in this app. A dead d.o endpoint (markdownify:
node.json answers 302 → 404) lands on GitLab through the same rule.
`search_suggestions` applies the same GitLab-first check to the machine
names it returns, so the Add-project search never offers a drupal.org link
for a project that is live on GitLab.

## Release

`.github/workflows/release.yml` (manual dispatch, patch/minor/major) bumps the
version everywhere via `scripts/sync-version.mjs`, tags, and pushes; that tag
triggers `.github/workflows/build-app.yml` (4-target matrix build via
tauri-action, publishes the GitHub release). Requires the `RELEASE_PAT`
secret. Cargo.toml version is the source of truth for bundle filenames.

## Conventions

- Rust: modules per concern (see layout), `se()`/`err_chain()` helpers,
  `eprintln!("[refresh] …")` instrumentation in `refresh_one` (visible when
  the binary is launched from a terminal).
- Frontend: components in `src/components/`, no state library — React hooks
  with `useCallback` loaders in `App.tsx`; UI strings plain English; dark
  theme via CSS variables in `App.css`.
- Package name is `drupal-issue-hub`; keep `scripts/sync-version.mjs`'s
  Cargo.lock regex in sync if the crate name ever changes.
