import { useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { api, type AssetLink, type Comment, type Issue } from "../api";
import { Icon } from "./Icon";
import { LabelEditor } from "./LabelEditor";
import { renderMarkup } from "../render";
import { dateStr, timeAgo, statusClass } from "../util";

const MIN_WIDTH = 320;
const MAX_WIDTH = 800;
const DEFAULT_WIDTH = 440;

/** drupal.org writes its issue templates as plain text, not markup: the body
 * arrives with "Problem/Motivation" and "Proposed resolution" as bold
 * paragraphs, so a 900-word description is one undifferentiated wall. These are
 * the headings worth jumping to, in the order drupal.org uses them. */
const SECTIONS = [
  "Problem/Motivation",
  "Steps to reproduce",
  "Proposed resolution",
  "Remaining tasks",
  "User interface changes",
  "API changes",
  "Data model changes",
  "Release notes snippet",
];

/** Beyond this a description is collapsed behind one click. */
const COLLAPSE_WORDS = 600;

const norm = (t: string) =>
  t.replace(/<[^>]+>/g, "").replace(/&amp;/g, "&").replace(/\s+/g, " ").trim().replace(/:$/, "").toLowerCase();

/**
 * Adds ids to the known section headings and reports what it found, so the pane
 * can offer a jump list. Runs on markup that DOMPurify has already sanitised and
 * only ever inserts an id this file chose - headings keep any id they had.
 */
function withSectionAnchors(html: string): {
  html: string;
  sections: { id: string; label: string }[];
} {
  const found: { id: string; label: string }[] = [];
  const anchor = (tag: string, attrs: string, inner: string) => {
    const hit = SECTIONS.find((s) => norm(inner) === norm(s));
    if (!hit) return null;
    const id = "sec-" + hit.toLowerCase().replace(/[^a-z0-9]+/g, "-");
    if (!found.some((f) => f.id === id)) found.push({ id, label: hit });
    const clean = attrs.replace(/\sid="[^"]*"/i, "");
    return `<${tag}${clean} id="${id}">${inner}</${tag}>`;
  };

  let out = html.replace(
    /<(h[1-6])([^>]*)>([\s\S]*?)<\/\1>/gi,
    (m, tag, attrs, inner) => anchor(tag, attrs, inner) ?? m,
  );
  // ...and the same names written as a bold paragraph, which is how drupal.org
  // issue bodies actually carry them: give those the heading they describe.
  out = out.replace(/<p>\s*(?:<strong>|<b>)?\s*([^<]{3,60}?)\s*(?:<\/strong>|<\/b>)?\s*<\/p>/gi, (m, text) => {
    if (!SECTIONS.some((s) => norm(s) === norm(text))) return m;
    const id = anchor("h3", ' class="section-head"', text) ?? null;
    return id ?? m;
  });

  return { html: out, sections: found };
}

function loadWidth(): number {
  const stored = Number(localStorage.getItem("diw_detail_width"));
  if (!Number.isFinite(stored) || stored === 0) return DEFAULT_WIDTH;
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, stored));
}

/** Click handler for rendered issue/comment HTML: route outbound links to
 * the system browser instead of navigating the webview. */
function outboundLinkHandler(e: React.MouseEvent<HTMLDivElement>) {
  const anchor = (e.target as HTMLElement).closest("a");
  if (!anchor) return;
  const href = anchor.getAttribute("href") ?? "";
  if (/^https?:\/\//i.test(href)) {
    e.preventDefault();
    void openUrl(href).catch(() => {});
  }
}

export function IssueDetail({
  issue,
  hasToken,
  gitlabUser,
  hideSystemComments,
  focused,
  onToggleFocus,
  onOpenSettings,
  onToggleFavorite,
  onUpdated,
  onClose,
}: {
  issue: Issue;
  hasToken: boolean;
  gitlabUser: string | null;
  hideSystemComments: boolean;
  /** Focus mode hides the list and gives the pane the whole window. */
  focused: boolean;
  onToggleFocus: () => void;
  onOpenSettings: () => void;
  onToggleFavorite: (issue: Issue) => void;
  onUpdated: () => void;
  onClose: () => void;
}) {
  const [width, setWidth] = useState(loadWidth);
  const widthRef = useRef(width);
  const [comment, setComment] = useState("");
  const [comments, setComments] = useState<Comment[] | null>(null);
  const [commentsError, setCommentsError] = useState<string | null>(null);
  const [commentSearch, setCommentSearch] = useState("");
  const [commentSort, setCommentSort] = useState<"asc" | "desc">("desc");
  const [assets, setAssets] = useState<AssetLink[] | null>(null);
  const [commentProgress, setCommentProgress] = useState<{
    done: number;
    total: number;
  } | null>(null);
  const [editing, setEditing] = useState(false);
  const [bodyExpanded, setBodyExpanded] = useState(false);
  const [title, setTitle] = useState(issue.title);
  const [body, setBody] = useState(issue.body);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editingNote, setEditingNote] = useState<string | null>(null);
  const [editingBody, setEditingBody] = useState("");
  const [confirmNote, setConfirmNote] = useState<string | null>(null);
  // The settings default can be overridden per thread from the discussion
  // header; opening another issue returns to the default.
  const [hideSystemOverride, setHideSystemOverride] = useState<boolean | null>(null);
  const hideSystem = hideSystemOverride ?? hideSystemComments;
  const lastNoteCount = useRef<number | null>(null);
  const wroteAt = useRef(0);

  const writable = issue.source === "gitlab" && hasToken;
  // GitLab allows only the author of a note to change or delete it, and system
  // notes are off-limits, so the buttons appear on your own comments only.
  const canModerate = (c: Comment) =>
    writable &&
    !c.system &&
    !!gitlabUser &&
    c.author.toLowerCase() === gitlabUser.toLowerCase();

  const rendered = useMemo(
    () => withSectionAnchors(renderMarkup(issue.source, issue.body)),
    [issue.body, issue.source],
  );
  const html = rendered.html;
  const bodyWords = useMemo(
    () => (issue.body.match(/\S+/g) || []).length,
    [issue.body],
  );
  const collapsible = bodyWords > COLLAPSE_WORDS;

  // Load the discussion thread and attached patches/MRs whenever another
  // issue is opened. Assets for GitLab scan the cached comments, so they are
  // fetched after the comments have been refreshed.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    setComments(null);
    setCommentsError(null);
    setAssets(null);
    setCommentProgress(null);
    setCommentSearch("");
    setEditingNote(null);
    setConfirmNote(null);
    setHideSystemOverride(null);
    setBodyExpanded(false);
    lastNoteCount.current = null;
    const load = async () => {
      try {
        const c = await api.getComments(issue.id);
        if (cancelled) return;
        setComments(c);
        setCommentProgress(null);
        try {
          setAssets(await api.getIssueAssets(issue.id));
        } catch {
          if (!cancelled) setAssets([]);
        }
      } catch (e) {
        if (!cancelled) setCommentsError(String(e));
      }
    };
    // Progress events arrive per chunk of uncached drupal.org comments.
    void listen<{ issueId: number; done: number; total: number }>(
      "comments-progress",
      (e) => {
        if (!cancelled && e.payload.issueId === issue.id) {
          setCommentProgress({ done: e.payload.done, total: e.payload.total });
        }
      },
    ).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    void load();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [issue.id]);

  // Hosts record status changes as notes, which bury the actual discussion when
  // they are switched off. Only the rendering hides them: the cache keeps every
  // note, and toggling the setting costs no requests.
  const shownComments = useMemo(
    () =>
      comments && hideSystem
        ? comments.filter((c) => !c.system)
        : comments,
    [comments, hideSystem],
  );
  const hiddenSystem =
    comments && hideSystem
      ? comments.length - (shownComments?.length ?? 0)
      : 0;
  const hasSystemNotes = !!comments?.some((c) => c.system);

  const visibleComments = useMemo(() => {
    if (!shownComments) return null;
    const q = commentSearch.trim().toLowerCase();
    const filtered = q
      ? shownComments.filter(
          (c) =>
            c.body.toLowerCase().includes(q) || c.author.toLowerCase().includes(q),
        )
      : shownComments;
    return commentSort === "desc" ? [...filtered].reverse() : filtered;
  }, [shownComments, commentSearch, commentSort]);

  const startResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = widthRef.current;
    const onMove = (ev: MouseEvent) => {
      const next = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startWidth + (startX - ev.clientX)));
      widthRef.current = next;
      setWidth(next);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      localStorage.setItem("diw_detail_width", String(widthRef.current));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  // Double-clicking the grip returns the pane to its default width, so a bad
  // drag is always one gesture from undone.
  const resetWidth = () => {
    widthRef.current = DEFAULT_WIDTH;
    setWidth(DEFAULT_WIDTH);
    localStorage.setItem("diw_detail_width", String(DEFAULT_WIDTH));
  };

  // Issue bodies link to drupal.org pages, patches and images — route
  // outbound links to the system browser instead of the webview.
  const onBodyClick = outboundLinkHandler;

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      onUpdated();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // Opening an issue always re-reads its notes from GitLab, so a write to one
  // of them is picked up by reloading the thread — and with it the MR links
  // that are scraped out of comment bodies. While the reload is in flight the
  // thread behind it is stale, so the pane shows that instead of pretending
  // the old comments are current.
  const [reloading, setReloading] = useState(false);
  const reloadThread = async () => {
    setReloading(true);
    try {
      setComments(await api.getComments(issue.id));
      try {
        setAssets(await api.getIssueAssets(issue.id));
      } catch {
        setAssets([]);
      }
    } finally {
      setReloading(false);
    }
  };

  const runThread = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      await reloadThread();
      wroteAt.current = Date.now();
      onUpdated();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // Escape closes the pane. Bubble phase on purpose: pickers, menus, popovers
  // and modals consume Escape in the capture phase first, so this only fires
  // when nothing more specific wanted the key.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  // Follow upstream activity while an issue stays open: comment_count is GitLab's
  // user_notes_count, so it moves whenever anyone adds or deletes a comment — a
  // delete is otherwise invisible until the issue is re-opened. drupal.org is
  // excluded on purpose, because its thread costs up to 70 requests and loads on
  // open only.
  useEffect(() => {
    if (issue.source !== "gitlab" || !hasToken) return;
    const seen = lastNoteCount.current;
    lastNoteCount.current = issue.comment_count;
    if (seen === null || seen === issue.comment_count) return;
    // Our own post/delete has just reloaded the thread; don't fetch it twice.
    if (Date.now() - wroteAt.current < 3000) return;
    // Reading notes needs the token, so a failure here is possible (revoked or
    // removed between refreshes) and belongs in the error strip, not the console.
    reloadThread().catch((e) => setError(String(e)));
  }, [issue.source, issue.comment_count, hasToken]);

  return (
    <aside
      className={`detail ${focused ? "focused" : ""}`}
      style={focused ? { width: "100%", minWidth: 0 } : { width, minWidth: width }}
    >
      {!focused && (
        <div
          className="resize-handle"
          onMouseDown={startResize}
          onDoubleClick={resetWidth}
          title="Drag to resize · double-click for the default width"
        />
      )}
      <div className="detail-content">
        <div className="detail-header">
          <button className="ghost small" onClick={onClose}>
            ← Back
          </button>
          <div className="detail-header-actions">
            <button
              className="ghost small"
              onClick={onToggleFocus}
              title={
                focused
                  ? "Bring the issue list back"
                  : "Hide the list and read this issue on its own"
              }
            >
              {focused ? "Exit focus" : "Focus"}
            </button>
            <button
              className={`ghost small star-btn ${issue.favorite ? "active" : ""}`}
              title={issue.favorite ? "Remove from favorites" : "Add to favorites"}
              onClick={() => onToggleFavorite(issue)}
            >
              <Icon name="star" size={13} filled={issue.favorite} />
            </button>
            <button
              className="ghost small"
              onClick={() => void openUrl(issue.url).catch(() => {})}
            >
              Open in browser ↗
            </button>
          </div>
        </div>
        <h2>{issue.title}</h2>
        <p className="meta dim">
          #{issue.ext_id} · {issue.source === "drupal" ? "drupal.org" : "git.drupalcode.org"}
        </p>
        <p className="meta">
          <span className={`pill ${statusClass(issue.status_label)}`}>
            {issue.status_label}
          </span>
          {issue.category_label && <span className="pill st-other">{issue.category_label}</span>}
          {issue.version && <span className="pill st-other">{issue.version}</span>}
        </p>
        <p className="meta dim">
          by {issue.author || "unknown"} · created {dateStr(issue.created_at)} · updated{" "}
          {timeAgo(issue.changed_at)}
        </p>
        {/* Labels are classification, so they sit with the classification, not at
            the bottom of a long scroll — where nobody reached them. */}
        {issue.source === "gitlab" && (
          <LabelEditor
            issue={issue}
            projectId={issue.project_id}
            editable={writable}
            onSaved={onUpdated}
          />
        )}

        <div className={`detail-scroll${reloading ? " busy" : ""}`}>
        {editing ? (
          <div className="edit-form">
            <input value={title} onChange={(e) => setTitle(e.target.value)} />
            <textarea
              rows={14}
              value={body}
              onChange={(e) => setBody(e.target.value)}
            />
            <div className="row">
              <button
                className="primary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await api.updateIssue(issue.id, title, body);
                    setEditing(false);
                  })
                }
              >
                Save changes
              </button>
              <button
                className="ghost"
                onClick={() => {
                  setEditing(false);
                  setTitle(issue.title);
                  setBody(issue.body);
                }}
              >
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <>
            {rendered.sections.length >= 2 && (
              <div className="section-jump">
                {rendered.sections.map((s) => (
                  <button
                    key={s.id}
                    className="ghost small"
                    onClick={() => {
                      // A collapsed body would hide the target.
                      setBodyExpanded(true);
                      requestAnimationFrame(() =>
                        document.getElementById(s.id)?.scrollIntoView({ block: "start" }),
                      );
                    }}
                  >
                    {s.label}
                  </button>
                ))}
              </div>
            )}
            <div
              className={collapsible && !bodyExpanded ? "body-clamp" : "body-open"}
            >
              <div
                className={`rendered-body${collapsible && !bodyExpanded ? " clamped" : ""}`}
                dangerouslySetInnerHTML={{ __html: html }}
                onClick={onBodyClick}
              />
              {collapsible && (
                <button
                  className="ghost small show-full"
                  onClick={() => setBodyExpanded((v) => !v)}
                >
                  {bodyExpanded
                    ? "Collapse description"
                    : `Show full description (${bodyWords} words)`}
                </button>
              )}
            </div>
          </>
        )}

        {assets && assets.length > 0 && (
          <div className="assets">
            <h3>Patches &amp; MRs</h3>
            {assets.map((a) => (
              <button
                key={a.url}
                className={`asset asset-${a.kind}`}
                title="Open in browser"
                onClick={() => void openUrl(a.url).catch(() => {})}
              >
                <span className="asset-icon">
                  <Icon name={a.kind === "patch" ? "file" : "merge"} size={13} />
                </span>
                {a.name}
              </button>
            ))}
          </div>
        )}

        <div className="discussion">
          <h3>
            {comments === null
              ? "Discussion"
              : hiddenSystem > 0
                ? `Discussion (${shownComments!.length} shown · ${comments.length} total)`
                : comments.length > 0
                  ? `Discussion (${comments.length})`
                  : "Discussion"}
          </h3>
          {shownComments && shownComments.length > 0 && (
            <div className="comment-toolbar">
              <input
                className="comment-search"
                placeholder="Search comments…"
                value={commentSearch}
                onChange={(e) => setCommentSearch(e.target.value)}
              />
              <button
                className="ghost small"
                title="Reverse the order"
                onClick={() =>
                  setCommentSort((s) => (s === "asc" ? "desc" : "asc"))
                }
              >
                {commentSort === "asc" ? "Sorted: oldest first" : "Sorted: newest first"}
              </button>
              {hasSystemNotes && (
                <button
                  className="ghost small comments-toggle"
                  onClick={() => setHideSystemOverride(!hideSystem)}
                  title="Status-change notes are host-generated ('Status changed to …'). Hiding them is display-only: the cache and notification counts are unaffected."
                >
                  {hideSystem
                    ? `Show ${hiddenSystem} status ${hiddenSystem === 1 ? "change" : "changes"}`
                    : "Hide status changes"}
                </button>
              )}
            </div>
          )}
          {comments === null && !commentsError && (
            <p className="hint">
              Loading comments…
              <span className="loading-dot" aria-label="loading" />
              {commentProgress && (
                <span className="refresh-progress">
                  <span className="progress-bar">
                    <span
                      className="progress-fill"
                      style={{
                        width: `${Math.round(
                          (commentProgress.done / Math.max(1, commentProgress.total)) * 100,
                        )}%`,
                      }}
                    />
                  </span>
                  {commentProgress.done}/{commentProgress.total}
                </span>
              )}
            </p>
          )}
          {commentsError && <p className="hint">{commentsError}</p>}
          {shownComments && shownComments.length === 0 && hiddenSystem > 0 && (
            <p className="hint">
              Only status-change notes here — {hiddenSystem} hidden. Show them
              with the button above.
            </p>
          )}
          {comments && comments.length === 0 && <p className="hint">No comments yet.</p>}
          {visibleComments && visibleComments.length === 0 && (shownComments?.length ?? 0) > 0 && (
            <p className="hint">No comments match “{commentSearch}”.</p>
          )}
          {visibleComments?.map((c) => {
            const mine = canModerate(c);
            const editing = editingNote === c.ext_id;
            const confirming = confirmNote === c.ext_id;
            return (
              <div key={c.ext_id} className={`comment ${c.system ? "system" : ""}`}>
                <p className="comment-meta dim">
                  {c.is_new && !c.system && <span className="badge new">NEW</span>}
                  <strong>{c.author || "unknown"}</strong> · {timeAgo(c.created_at)}
                  {c.system && " · status change"}
                  {mine && !editing && !confirming && (
                    <span className="comment-actions">
                      <button
                        className="ghost small"
                        title="Edit your comment"
                        disabled={busy}
                        onClick={() => {
                          setError(null);
                          setConfirmNote(null);
                          setEditingNote(c.ext_id);
                          setEditingBody(c.body);
                        }}
                      >
                        <Icon name="pencil" size={13} />
                      </button>
                      <button
                        className="ghost small"
                        title="Delete your comment"
                        disabled={busy}
                        onClick={() => {
                          setError(null);
                          setEditingNote(null);
                          setConfirmNote(c.ext_id);
                        }}
                      >
                        <Icon name="trash" size={13} />
                      </button>
                    </span>
                  )}
                </p>
                {editing ? (
                  <div className="edit-form">
                    <textarea
                      rows={6}
                      value={editingBody}
                      disabled={busy}
                      onChange={(e) => setEditingBody(e.target.value)}
                    />
                    <div className="row">
                      <button
                        className="primary"
                        disabled={busy || !editingBody.trim()}
                        onClick={() =>
                          void runThread(async () => {
                            await api.editIssueComment(issue.id, c.ext_id, editingBody);
                            setEditingNote(null);
                          })
                        }
                      >
                        Save comment
                      </button>
                      <button
                        className="ghost"
                        disabled={busy}
                        onClick={() => setEditingNote(null)}
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                ) : (
                  <div
                    className="rendered-body comment-body"
                    dangerouslySetInnerHTML={{ __html: renderMarkup(issue.source, c.body) }}
                    onClick={outboundLinkHandler}
                  />
                )}
                {confirming && (
                  <div className="row comment-confirm">
                    <span className="hint">
                      Delete this comment on git.drupalcode.org? It is gone for
                      everyone.
                    </span>
                    <button
                      className="danger small"
                      disabled={busy}
                      onClick={() =>
                        void runThread(async () => {
                          await api.deleteIssueComment(issue.id, c.ext_id);
                          setConfirmNote(null);
                        })
                      }
                    >
                      Delete
                    </button>
                    <button
                      className="ghost small"
                      disabled={busy}
                      onClick={() => setConfirmNote(null)}
                    >
                      Cancel
                    </button>
                  </div>
                )}
              </div>
            );
          })}
        </div>

          {issue.source !== "gitlab" && (
            <p className="hint">
              drupal.org issue queues have no write API, so issues here are
              read-only. To edit an issue, follow the project's
              git.drupalcode.org work items instead.
            </p>
          )}
        </div>

        {issue.source === "gitlab" && (
          <div className="actions">
            {/* No heading: a textarea saying "Write a comment (Markdown)…" is
                not improved by the word "tools". */}
            {!hasToken && (
              <p className="hint">
                Add a git.drupalcode.org personal access token in Settings to
                comment on and edit this issue.
                <button className="ghost small" onClick={onOpenSettings}>
                  Open settings
                </button>
              </p>
            )}
            <textarea
              placeholder={writable ? "Write a comment (Markdown)…" : "Configure a token to comment"}
              rows={4}
              disabled={!writable}
              value={comment}
              onChange={(e) => setComment(e.target.value)}
            />
            <div className="row">
              <button
                className="primary"
                disabled={!writable || busy || !comment.trim()}
                onClick={() =>
                  void runThread(async () => {
                    await api.addIssueComment(issue.id, comment);
                    setComment("");
                  })
                }
              >
                Post comment
              </button>
              {writable && (
                <>
                  <button className="ghost" disabled={busy} onClick={() => setEditing(true)}>
                    Edit issue
                  </button>
                  <button
                    className="ghost"
                    disabled={busy}
                    onClick={() => void run(() => api.setIssueState(issue.id, issue.status_label !== "Closed"))}
                  >
                    {issue.status_label === "Closed" ? "Reopen issue" : "Mark closed"}
                  </button>
                </>
              )}
            </div>
          </div>
        )}

        {busy && <p className="hint">Sending…</p>}
        {error && <p className="error">{error}</p>}
      </div>
    </aside>
  );
}
