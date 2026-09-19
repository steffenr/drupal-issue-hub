import { useEffect, useMemo, useRef, useState } from "react";
import type { Issue } from "../api";
import { Icon } from "./Icon";
import { timeAgo, statusClass } from "../util";

type ColumnKey = "status" | "category" | "updated";
type SortKey = ColumnKey | "title";

/* Version was dropped: this ecosystem spells versions as free labels
 * (v2.4.x-dev), so the column measured 0/60 filled - permanently. Author
 * moved into the title cell for the same reason: a 130px column for one
 * short word was the width the detail pane kept eating (see App.tsx). */
const COLUMNS: { key: ColumnKey; label: string; width: string }[] = [
  { key: "status", label: "Status", width: "170px" },
  { key: "category", label: "Category", width: "90px" },
  { key: "updated", label: "Updated", width: "80px" },
];

function loadHidden(): Set<ColumnKey> {
  try {
    const raw = JSON.parse(localStorage.getItem("diw_hidden_columns") ?? "");
    // No stored preference yet: start from the default, which keeps the
    // Category column hidden because drupal.org queues carry only a handful
    // of distinct categories across hundreds of rows - a permanently sparse
    // column (same reason Version was dropped above). The user's explicit
    // choice is written back on any toggle, so it governs from then on.
    if (raw === "") return new Set<ColumnKey>(["category"]);
    return new Set(Array.isArray(raw) ? raw : []);
  } catch {
    return new Set<ColumnKey>(["category"]);
  }
}

export function IssueTable({
  issues,
  openIssueId,
  showProject,
  scopeKey,
  mode,
  onMode,
  scopeIssueCount,
  onOpen,
  onToggleFavorite,
  refreshing,
  progress,
  onRefresh,
  showLoadMore,
  hasMore,
  loadingMore,
  onLoadMore,
  scopeIsGitlabProject,
  scopeName,
  onCreateIssue,
  issueError,
  clearIssueError,
  onOpenIssueByIid,
  onCreateError,
}: {
  issues: Issue[];
  openIssueId: number | null;
  showProject: boolean;
  /** The selected project / favorites / all. Search and dropdown filters are
   * client-side over the loaded rows, so they must not survive a scope change:
   * carrying "translation" into the next project reads as an empty queue. */
  scopeKey: number | "favorites" | null;
  /** attention = the unseen/changed delta, all = the whole cached queue. */
  mode: "attention" | "all";
  onMode: (m: "attention" | "all") => void;
  /** How many issues the scope holds in total. In attention mode the server
   * sends zero rows when everything is read, so without this the empty state
   * cannot tell "nothing tracked" from "nothing new" - and the load-more
   * control cannot say how much of the queue is on screen. */
  scopeIssueCount: number;
  onOpen: (issue: Issue) => void;
  onToggleFavorite: (issue: Issue) => void;
  refreshing: boolean;
  progress: { done: number; total: number; name: string } | null;
  onRefresh: () => void;
  showLoadMore: boolean;
  hasMore: boolean | null;
  loadingMore: boolean;
  onLoadMore: () => void;
  /** true only when the scope is exactly one GitLab project: drupal.org
   * queues have no write API and "all projects" has no single target. */
  scopeIsGitlabProject: boolean;
  scopeName: string;
  /** Create a new issue on the scoped project, resolved to the new iid. */
  onCreateIssue: (title: string, description: string) => Promise<string>;
  issueError: string | null;
  clearIssueError: () => void;
  /** Open the just-created work item by its iid after the list reloaded. */
  onOpenIssueByIid: (iid: string) => void;
  /** Report a create failure back to App so it can show the error banner. */
  onCreateError: (message: string) => void;
}) {
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState("");
  const [category, setCategory] = useState("");
  const [formOpen, setFormOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [newBody, setNewBody] = useState("");
  const [hidden, setHidden] = useState<Set<ColumnKey>>(loadHidden);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  // Click a header to sort the loaded rows, again to reverse, a third time to
  // return to the default newest-changed order the backend ships.
  const [sort, setSort] = useState<{ key: SortKey; dir: 1 | -1 } | null>(null);

  const toggleSort = (key: SortKey) =>
    setSort((s) =>
      s?.key === key ? (s.dir === 1 ? { key, dir: -1 } : null) : { key, dir: 1 },
    );

  // The list can hold hundreds of rows; when a different row becomes the
  // open one, bring it into the visible part of the scroll container.
  //
  // Two traps make this non-obvious:
  //  1. The scroll container is .table, which clips a STICKY thead
  //     (z-index: 1, opaque background). A normal row can never sit behind
  //     it: overflow-y:auto forces visible-overflow semantics where the top
  //     of the content is always clipped and the thead floats above it.
  //     So "in view" means "fully below the thead AND inside the bottom
  //     edge" - measured with the thead's real height, not a fixed margin.
  //  2. The .selected class is added in the same render as the click
  //     handler's state update, so a scroll set synchronously there is
  //     measured against the pre-paint layout and gets overridden when the
  //     row reflows. One rAF after the class has painted is the earliest
  //     safe moment.
  const lastScrolledRef = useRef<number | null>(null);
  useEffect(() => {
    if (openIssueId == null) { lastScrolledRef.current = null; return; }
    if (lastScrolledRef.current === openIssueId) return;
    lastScrolledRef.current = openIssueId;
    const frame = requestAnimationFrame(() => {
      const el = document.querySelector('.trow[aria-current="true"]') as HTMLElement | null;
      if (!el) return;
      const c = el.closest(".table") as HTMLElement | null;
      if (!c) return;
      const theadH = (c.querySelector(".thead") as HTMLElement | null)?.offsetHeight ?? 0;
      const cr = c.getBoundingClientRect();
      const er = el.getBoundingClientRect();
      const rowH = el.offsetHeight;
      // Offset of the row's top edge within the scrollable content, relative
      // to where the thead ends (the first place content can actually be seen).
      const contentTop = c.scrollTop + (er.top - cr.top) - theadH;
      const visibleH = c.clientHeight - theadH;
      const topOverflow = -contentTop; // how far the row pokes above the thead
      const bottomOverflow = contentTop + rowH - visibleH; // how far past the bottom
      if (topOverflow > 0) {
        c.scrollTop -= topOverflow;
      } else if (bottomOverflow > 0) {
        c.scrollTop += bottomOverflow;
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [openIssueId]);

  // Close the column context menu on Escape - in the capture phase, so the
  // menu consumes the key before the detail pane's own Escape handler and
  // opening a menu never closes the pane behind it.
  useEffect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        e.preventDefault();
        setMenu(null);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [menu]);

  const toggleColumn = (key: ColumnKey) => {
    setHidden((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      localStorage.setItem("diw_hidden_columns", JSON.stringify([...next]));
      return next;
    });
  };

  // New-issue form: create, then open the new work item by iid. The list
  // reload happens in App after the create; the iid is what the backend
  // returns, and the row lands in the cache before the open attempt.
  const submitNewIssue = async () => {
    if (!newTitle.trim() || submitting) return;
    setSubmitting(true);
    clearIssueError();
    try {
      const iid = await onCreateIssue(newTitle.trim(), newBody);
      // The form is armed again by a fresh "New issue" click; leaving it
      // open after a successful create reads as "still in progress".
      setFormOpen(false);
      setNewTitle("");
      setNewBody("");
      setSubmitting(false);
      onOpenIssueByIid(iid);
    } catch (e) {
      setSubmitting(false);
      onCreateError(String(e));
    }
  };

  // A new scope starts with a clean filter state.
  // Closing a scope also closes the new-issue form so a half-typed title
  // does not survive a project switch.
  useEffect(() => {
    setFormOpen(false);
    setNewTitle("");
    setNewBody("");
    clearIssueError();
  }, [scopeKey]);

  useEffect(() => {
    setSearch("");
    setStatus("");
    setCategory("");
  }, [scopeKey]);

  const filtersActive = search !== "" || status !== "" || category !== "";
  const clearFilters = () => {
    setSearch("");
    setStatus("");
    setCategory("");
  };

  const statuses = useMemo(
    () => Array.from(new Set(issues.map((i) => i.status_label))).sort(),
    [issues],
  );
  const categories = useMemo(
    () =>
      Array.from(new Set(issues.map((i) => i.category_label).filter(Boolean))).sort(),
    [issues],
  );

  // Category ships hidden by default (see loadHidden) because the column is
  // permanently sparse on drupal.org queues; the user's Columns choice is the
  // single source of truth thereafter - a ticked box is never overridden by
  // the row count, which is what made "select Category" silently no-op before.
  const visible = COLUMNS.filter((c) => !hidden.has(c.key));
  // Attention mode is the triage pass: the delta, three columns, nothing else.
  const shown = mode === "attention" ? visible.filter((c) => c.key !== "category") : visible;
  const gridTemplate = `minmax(240px, 1fr) ${shown.map((c) => c.width).join(" ")}`;

  const filtered = issues.filter((i) => {
    if (status && i.status_label !== status) return false;
    if (category && i.category_label !== category) return false;
    if (search) {
      const q = search.toLowerCase();
      if (
        !i.title.toLowerCase().includes(q) &&
        !i.author.toLowerCase().includes(q) &&
        !i.project_name.toLowerCase().includes(q) &&
        // The number is how a Drupal issue is cited everywhere else - links,
        // e-mail, IRC - so it must find the row it belongs to.
        !i.ext_id.toLowerCase().includes(q)
      )
        return false;
    }
    return true;
  });

  const sortKeyOf = (i: Issue): string | number =>
    sort?.key === "title"
      ? i.title.toLowerCase()
      : sort?.key === "updated"
        ? i.changed_at
        : sort?.key === "status"
          ? i.status_label.toLowerCase()
          : sort?.key === "category"
            ? i.category_label.toLowerCase()
            : "";
  const sorted = sort
    ? [...filtered].sort((a, b) => {
        const va = sortKeyOf(a);
        const vb = sortKeyOf(b);
        const cmp =
          typeof va === "number" && typeof vb === "number"
            ? va - vb
            : String(va).localeCompare(String(vb));
        return cmp * sort.dir;
      })
    : filtered;

  const cell = (issue: Issue, key: ColumnKey) => {
    switch (key) {
      case "status":
        return (
          <span className={`pill ${statusClass(issue.status_label)}`}>
            {issue.status_label}
          </span>
        );
      case "category":
        return issue.category_label;
      case "updated":
        return <span className="dim">{timeAgo(issue.changed_at)}</span>;
    }
  };

  return (
    <section className="issues">
      <div className="toolbar">
        <div className="mode-switch" role="group" aria-label="Which issues to show">
          <button
            className={mode === "attention" ? "on" : ""}
            title="Issues that are new or changed since you last looked"
            onClick={() => onMode("attention")}
          >
            Needs attention
          </button>
          <button
            className={mode === "all" ? "on" : ""}
            title="Everything cached for this scope"
            onClick={() => onMode("all")}
          >
            All
          </button>
        </div>
        <input
          className="search"
          placeholder="Search title, number, author, project…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <select value={status} onChange={(e) => setStatus(e.target.value)}>
          <option value="">All statuses</option>
          {statuses.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <select value={category} onChange={(e) => setCategory(e.target.value)}>
          <option value="">All categories</option>
          {categories.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        {filtersActive && (
          <button className="ghost small" title="Reset search and dropdown filters" onClick={clearFilters}>
            Clear filters
          </button>
        )}
        {refreshing && progress && progress.total > 0 && (
          <span className="refresh-progress" title={`Refreshing ${progress.name}`}>
            <span className="progress-bar">
              <span
                className="progress-fill"
                style={{ width: `${Math.round((progress.done / progress.total) * 100)}%` }}
              />
            </span>
            <span className="dim">
              {progress.done}/{progress.total} · {progress.name}
            </span>
          </span>
        )}
        <button
          className="ghost"
          onClick={onRefresh}
          disabled={refreshing}
          title="Refresh every project (⌘R)"
        >
          {refreshing ? "Refreshing…" : (<>
            <Icon name="refresh" size={13} /> Refresh
          </>)}
        </button>
        {scopeIsGitlabProject && (
          <button className="ghost" onClick={() => setFormOpen((v) => !v)} title={`Create a new work item on ${scopeName}`}>
            <Icon name="plus" size={13} /> New issue
          </button>
        )}
      </div>

      {scopeIsGitlabProject && formOpen && (
        <div className="new-issue-form">
          <input
            className="new-issue-title"
            placeholder={`New issue on ${scopeName}…`}
            value={newTitle}
            onChange={(e) => setNewTitle(e.target.value)}
            disabled={submitting}
            autoFocus
            onKeyDown={(e) => {
              // Cmd/Ctrl+Enter submits from the title field, the way the
              // detail pane's comment box already does.
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") void submitNewIssue();
            }}
          />
          <textarea
            className="new-issue-body"
            placeholder="Description (markdown)"
            rows={3}
            value={newBody}
            onChange={(e) => setNewBody(e.target.value)}
            disabled={submitting}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") void submitNewIssue();
            }}
          />
          {issueError && <p className="hint error-inline">{issueError}</p>}
          <div className="new-issue-actions">
            <button
              className="primary"
              disabled={submitting || !newTitle.trim()}
              onClick={() => void submitNewIssue()}
              type="button"
            >
              {submitting ? "Creating…" : "Create issue"}
            </button>
            <button className="ghost" disabled={submitting} onClick={() => setFormOpen(false)} type="button">
              Cancel
            </button>
            <span className="hint dim" title="Same as the comment box in the issue view.">⌘/Ctrl + Enter to submit</span>
          </div>
        </div>
      )}

      <div className="table">
        <div
          className="thead"
          style={{ gridTemplateColumns: gridTemplate }}
          onContextMenu={(e) => {
            // The context menu on a header is the column switch, so opening
            // it from a row right-click shows it at the header's place
            // instead of wherever the pointer happens to be.
            e.preventDefault();
            const t = e.target as HTMLElement;
            const head = t.closest(".thead")?.querySelector(".head-cell") ?? null;
            const r = head ? head.getBoundingClientRect() : t.getBoundingClientRect();
            setMenu({ x: r.right - 156, y: r.bottom + 4 });
          }}
        >
          <button
            className="head-cell col-title"
            onClick={() => toggleSort("title")}
            aria-sort={sort?.key === "title" ? (sort.dir === 1 ? "ascending" : "descending") : undefined}
          >
            Issue{sort?.key === "title" ? (sort.dir === 1 ? " ↑" : " ↓") : ""}
          </button>
          {shown.map((c) => (
            <button
              key={c.key}
              className="head-cell"
              onClick={() => toggleSort(c.key)}
              aria-sort={sort?.key === c.key ? (sort.dir === 1 ? "ascending" : "descending") : undefined}
            >
              {c.label}
              {sort?.key === c.key ? (sort.dir === 1 ? " ↑" : " ↓") : ""}
            </button>
          ))}
        </div>
        {sorted.map((i) => (
          <div
            key={i.id}
            className={`trow ${i.id === openIssueId ? "selected" : ""}`}
            style={{ gridTemplateColumns: gridTemplate }}
            aria-current={i.id === openIssueId ? "true" : undefined}
            role="button"
            tabIndex={0}
            onClick={() => onOpen(i)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onOpen(i);
              }
            }}
          >
            <span className="col-title">
              <span className="title-line">
                {i.is_new && <span className="badge new">NEW</span>}
                {i.has_changes && !i.is_new && <span className="badge changed">UPD</span>}
                <span className="iid dim">#{i.ext_id}</span>
                <span className="title">{i.title}</span>
              </span>
              <span className="meta-line">
                {i.author && <span className="author">{i.author}</span>}
                {i.comment_count > 0 && (
                  <span className="cmt">
                    <Icon name="comment" size={11} /> {i.comment_count}
                  </span>
                )}
                {/* Favorite is a marker, not an affordance on the title line:
                    it lives with the other markers, and when a row has no
                    comments it still has a home instead of a gap. */}
                <button
                  className={`star-btn ${i.favorite ? "active" : ""}`}
                  title={i.favorite ? "Remove from favorites" : "Add to favorites"}
                  onClick={(e) => {
                    e.stopPropagation();
                    onToggleFavorite(i);
                  }}
                >
                  <Icon name="star" size={12} filled={i.favorite} />
                </button>
                {showProject && <span className="project">— {i.project_name}</span>}
              </span>
            </span>
            {shown.map((c) => (
              <span key={c.key} className={`col-${c.key}`}>
                {cell(i, c.key)}
              </span>
            ))}
          </div>
        ))}
        {showLoadMore && hasMore !== false && (
          <div className="load-more">
            <button className="ghost" disabled={loadingMore} onClick={onLoadMore}>
              {loadingMore
                ? "Loading older issues…"
                : scopeIssueCount > issues.length
                  ? `Load more issues (${issues.length} of ${scopeIssueCount})`
                  : "Load more issues"}
            </button>
          </div>
        )}
        {filtered.length === 0 && (
          <p className="empty">
            {!scopeIssueCount
              ? "No issues tracked yet — refresh or add a project."
              : mode === "attention"
                ? "Nothing needs attention — everything here is read. Switch to All to browse the queue."
                : "No issues match the current filters."}
          </p>
        )}
      </div>
      <p className="count dim">
        {filtered.length} of {issues.length} issues
      </p>

      {menu && (
        <>
          <div className="popover-backdrop" onClick={() => setMenu(null)} onContextMenu={(e) => { e.preventDefault(); setMenu(null); }} />
          <div
            className="popover"
            role="menu"
            aria-label="Toggle columns"
            style={{
              left: Math.min(menu.x, window.innerWidth - 180),
              top: Math.min(menu.y, window.innerHeight - 220),
            }}
          >
            {COLUMNS.map((c) => (
              <label key={c.key} className="check" role="menuitemcheckbox" aria-checked={!hidden.has(c.key)}>
                <input
                  type="checkbox"
                  checked={!hidden.has(c.key)}
                  onChange={() => toggleColumn(c.key)}
                />
                {c.label}
              </label>
            ))}
          </div>
        </>
      )}
    </section>
  );
}
