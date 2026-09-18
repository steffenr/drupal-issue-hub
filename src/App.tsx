import { useCallback, useEffect, useRef, useState } from "react";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { api, type Issue, type Project, type Settings } from "./api";
import { timeAgo } from "./util";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Sidebar } from "./components/Sidebar";
import { IssueTable } from "./components/IssueTable";
import { IssueDetail } from "./components/IssueDetail";
import { AddProjectModal } from "./components/AddProjectModal";
import { SettingsModal } from "./components/SettingsModal";
import "./App.css";

/**
 * How long a parked notification target is worth jumping to. A banner older
 * than this has been overtaken by whatever the next poll reports, and the NEW
 * badges still carry it.
 */
const NOTIFICATION_JUMP_TTL_MS = 120_000;

function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [selectedId, setSelectedId] = useState<number | "favorites" | null>(null);
  const [favCount, setFavCount] = useState(0);
  const [openIssue, setOpenIssue] = useState<Issue | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [deleteCandidate, setDeleteCandidate] = useState<Project | null>(null);
  // Which list the table shows: the delta since the last look, or everything.
  // attention is the default wherever there is something new to see.
  const [mode, setMode] = useState<"attention" | "all">("attention");
  // Reading mode: the pane takes the window, the list steps aside but stays
  // mounted, so filters and scroll position survive the trip.
  const [focusDetail, setFocusDetail] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshProgress, setRefreshProgress] = useState<{
    done: number;
    total: number;
    name: string;
  } | null>(null);
  const [hasMore, setHasMore] = useState<boolean | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);

  const loadProjects = useCallback(async () => {
    setProjects(await api.listProjects());
    setFavCount(await api.favoritesCount());
  }, []);

  const loadIssues = useCallback(async () => {
    const favoritesOnly = selectedId === "favorites";
    const list = await api.listIssues(
      selectedId === "favorites" ? 0 : (selectedId ?? 0),
      mode === "attention",
      favoritesOnly,
    );
    setIssues(list);
    // The detail pane keeps a snapshot of the row it opened; refresh it from the
    // new list so its comment count — and the thread reload driven by that
    // count — stays current while the pane stays open. When a filter drops the
    // row from the list, leave the open issue alone instead of closing it.
    setOpenIssue((open) => {
      if (!open) return open;
      const fresh = list.find((i) => i.id === open.id);
      return fresh ? { ...fresh, favorite: open.favorite } : open;
    });
    // "Probably more on the server" heuristic until a load-more call resolves
    // it: a full first page per source size.
    if (typeof selectedId === "number") {
      const pageSize = list[0]?.source === "gitlab" ? 100 : 50;
      setHasMore(list.length >= pageSize);
    } else {
      setHasMore(null);
    }
  }, [selectedId, mode]);

  const loadSettings = useCallback(async () => {
    setSettings(await api.getSettings());
  }, []);

  useEffect(() => {
    void loadProjects();
    void loadSettings();
  }, [loadProjects, loadSettings]);

  useEffect(() => {
    void loadIssues();
  }, [loadIssues]);

  // The starting filter is a setting, not a guess: "attention" where there is
  // something new to triage, "all" to read the queue (the old behaviour). A
  // change here re-applies the default when the scope switches, without
  // yanking the view out from under a reader on later refreshes. The effect
  // may only fire once the projects list has arrived.
  const projectsLoaded = projects.length > 0;
  const defaultMode = settings?.default_mode ?? "attention";
  useEffect(() => {
    if (projectsLoaded) setMode(defaultMode);
  }, [selectedId, projectsLoaded, defaultMode]);

  // A monitor that does not say when it last looked is indistinguishable from
  // a broken one, so the topbar reports staleness. Ticks twice a minute.
  const [nowSec, setNowSec] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const t = setInterval(() => setNowSec(Math.floor(Date.now() / 1000)), 30_000);
    return () => clearInterval(t);
  }, []);
  const freshnessProps = {
    pollEnabled: !!settings?.poll_enabled,
    pollIntervalMinutes: settings?.poll_interval_minutes ?? 15,
  };

  // Checkup on opening a project: refresh its first page so new issues and
  // comment counts are current. Runs once per opened project.
  const checkedRef = useRef<number | null>(null);
  useEffect(() => {
    if (selectedId === null || selectedId === "favorites") {
      checkedRef.current = null;
      return;
    }
    if (checkedRef.current === selectedId) return;
    checkedRef.current = selectedId;
    void (async () => {
      setRefreshing(true);
      try {
        await api.refreshProject(selectedId);
      } finally {
        setRefreshing(false);
      }
      await Promise.all([loadProjects(), loadIssues()]);
    })();
  }, [selectedId, loadProjects, loadIssues]);

  const loadMore = async () => {
    if (typeof selectedId !== "number" || loadingMore) return;
    setLoadingMore(true);
    try {
      const res = await api.loadMoreIssues(selectedId);
      await loadIssues();
      setHasMore(res.hasMore);
    } finally {
      setLoadingMore(false);
    }
  };

  const refreshAll = useCallback(
    async (notify: boolean) => {
      setRefreshing(true);
      try {
        // Refresh project by project so progress can be reported per project.
        const all = await api.listProjects();
        const results = [];
        for (let i = 0; i < all.length; i++) {
          setRefreshProgress({
            done: i,
            total: all.length,
            name: all[i].name,
          });
          results.push(await api.refreshProject(all[i].id));
        }
        setRefreshProgress(null);
        // One banner per project, carrying only the signals that project opted
        // in to. "Updated" issues deliberately never notify: on drupal.org a
        // status assignment bumps both `changed` and the comment count, so it
        // would surface as a comment notification anyway.
        if (notify) {
          type Banner = {
            opts: { title: string; body: string };
            projectId: number;
            issueId: number | null;
          };
          const banners: Banner[] = results.flatMap((r) => {
            if (!r.ok) return [];
            const parts: string[] = [];
            if (r.notify_new_issues && r.new_issues > 0)
              parts.push(`${r.new_issues} new issue${r.new_issues === 1 ? "" : "s"}`);
            if (r.notify_new_comments && r.new_comments > 0)
              parts.push(
                `${r.new_comments} new comment${r.new_comments === 1 ? "" : "s"}`,
              );
            return parts.length
              ? [
                  {
                    opts: { title: r.project_name, body: parts.join(" · ") },
                    projectId: r.project_id,
                    issueId: r.target_issue_id,
                  },
                ]
              : [];
          });
          if (banners.length > 0) {
            // Ask for permission once for the whole batch, not per project.
            let granted = await isPermissionGranted();
            if (!granted) granted = (await requestPermission()) === "granted";
            if (granted) {
              for (const banner of banners) await sendNotification(banner.opts);
              // Park the target so the activation this banner causes can use
              // it. When several projects reported at once the last one with a
              // target wins — the activation signal carries no identity, so
              // only one jump is possible; the others keep their NEW badges.
              for (const banner of banners) {
                if (banner.issueId !== null) {
                  pendingJump.current = {
                    projectId: banner.projectId,
                    issueId: banner.issueId,
                    at: Date.now(),
                  };
                }
              }
            }
          }
        }
        await Promise.all([loadProjects(), loadIssues()]);
      } finally {
        setRefreshing(false);
      }
    },
    [loadProjects, loadIssues],
  );

  // Background polling: re-arm whenever settings change.
  const pollTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  useEffect(() => {
    if (pollTimer.current) clearInterval(pollTimer.current);
    if (settings?.poll_enabled && settings.poll_interval_minutes > 0) {
      pollTimer.current = setInterval(
        () => void refreshAll(true),
        settings.poll_interval_minutes * 60_000,
      );
    }
    return () => {
      if (pollTimer.current) clearInterval(pollTimer.current);
    };
  }, [settings, refreshAll]);

  const openIssueRow = async (issue: Issue) => {
    setOpenIssue(issue);    if (issue.is_new || issue.has_changes) {
      try {
        await api.markIssueSeen(issue.id);
        await Promise.all([loadIssues(), loadProjects()]);
        // Keep the panel content but with cleared flags.
        setOpenIssue({ ...issue, is_new: false, has_changes: false });
      } catch {
        /* non-fatal */
      }
    }
  };

  // Following a notification. The plugin cannot report a banner click —
  // tauri-plugin-notification 2.4.0 posts through notify-rust's
  // NSUserNotification backend, which has no activation delegate — so window
  // activation stands in for it: that is what clicking a banner causes. It
  // stays quiet when the app is already frontmost, because then there is
  // nothing to bring forward.
  const pendingJump = useRef<{
    projectId: number;
    issueId: number;
    at: number;
  } | null>(null);
  const openIssueRef = useRef(openIssueRow);
  openIssueRef.current = openIssueRow;

  useEffect(() => {
    const jump = async () => {
      const pending = pendingJump.current;
      if (!pending) return;
      // Consume first: two focus signals can arrive for one activation.
      pendingJump.current = null;
      if (Date.now() - pending.at > NOTIFICATION_JUMP_TTL_MS) return;
      try {
        const issue = await api.getIssue(pending.issueId);
        // Deleted, or moved to another project, since the banner fired.
        if (!issue || issue.project_id !== pending.projectId) return;
        setSelectedId(issue.project_id);
        await openIssueRef.current(issue);
      } catch {
        /* a failed lookup just means no jump */
      }
    };
    // Both signals are wired because either can be the one that fires, and the
    // first consumes the target so the second is a no-op.
    window.addEventListener("focus", jump);
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void jump();
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        /* no window event available: the DOM focus listener stands alone */
      });
    return () => {
      window.removeEventListener("focus", jump);
      unlisten?.();
    };
  }, []);

  // The delete modal has no focused input to take Escape, and the pane behind
  // it must not close instead - so it consumes the key in the capture phase.
  useEffect(() => {
    if (!deleteCandidate) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        e.preventDefault();
        setDeleteCandidate(null);
      }
    };
    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [deleteCandidate]);

  // The app's own shortcuts. The native menu carries the system roles (cut,
  // copy, paste, quit); these are the ones only this app can mean.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const k = e.key.toLowerCase();
      if (k === "r") {
        e.preventDefault();
        void refreshAll(false);
      } else if (k === ",") {
        e.preventDefault();
        setShowSettings(true);
      } else if (k === "0") {
        e.preventDefault();
        setSelectedId(null);
        setOpenIssue(null);
      } else if (e.shiftKey && k === "f") {
        e.preventDefault();
        setSelectedId("favorites");
        setOpenIssue(null);
      } else if (/^[1-9]$/.test(k)) {
        const p = projects[Number(k) - 1];
        if (p) {
          e.preventDefault();
          setSelectedId(p.id);
          setOpenIssue(null);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [projects, refreshAll]);

  // j/k walks the visible rows once one of them has focus, so a triage pass
  // needs no mouse. Ignored while typing.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key !== "j" && e.key !== "k") return;
      const el = e.target as HTMLElement | null;
      if (el?.closest("input, textarea, select")) return;
      const rows = Array.from(document.querySelectorAll<HTMLElement>(".trow"));
      if (rows.length === 0) return;
      const from = rows.indexOf(document.activeElement as HTMLElement);
      const next =
        from < 0
          ? 0
          : Math.min(rows.length - 1, Math.max(0, from + (e.key === "j" ? 1 : -1)));
      e.preventDefault();
      rows[next].focus();
      rows[next].scrollIntoView({ block: "nearest" });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const removeProject = async (p: Project) => {
    // NOTE: window.confirm is not implemented in Tauri's WKWebView and always
    // returns false on macOS — confirmation must be an in-app modal.
    await api.removeProject(p.id);
    if (selectedId === p.id) setSelectedId(null);
    if (openIssue && openIssue.project_id === p.id) setOpenIssue(null);
    await loadProjects();
  };

  const renameProject = async (p: Project, name: string) => {
    await api.renameProject(p.id, name);
    await loadProjects();
  };

  const setProjectNotifications = async (
    p: Project,
    notifyNewIssues: boolean,
    notifyNewComments: boolean,
  ) => {
    await api.setProjectNotifications(p.id, notifyNewIssues, notifyNewComments);
    // Reload so the sidebar bell reflects the stored state immediately.
    await loadProjects();
  };

  const reorderProjects = async (ids: number[]) => {
    await api.reorderProjects(ids);
    await loadProjects();
  };

  const toggleFavorite = async (issue: Issue) => {
    const isFav = await api.toggleFavorite(issue.id);
    setFavCount(await api.favoritesCount());
    // Update the row in place; drop it from view when the favorites filter is
    // active and it just got unstarred.
    setIssues((prev) =>
      prev
        .map((i) => (i.id === issue.id ? { ...i, favorite: isFav } : i))
        .filter((i) => selectedId !== "favorites" || i.favorite),
    );
    setOpenIssue((open) =>
      open && open.id === issue.id ? { ...open, favorite: isFav } : open,
    );
  };

  const selectedProject =
    selectedId === "favorites" ? null : (projects.find((p) => p.id === selectedId) ?? null);

  const refreshTimes = (selectedProject
    ? [selectedProject.last_refreshed]
    : projects.map((p) => p.last_refreshed)
  ).filter((t): t is number => typeof t === "number");
  const lastRefresh = refreshTimes.length ? Math.max(...refreshTimes) : null;
  const intervalSec = (settings?.poll_interval_minutes ?? 15) * 60;
  const staleness = !settings?.poll_enabled
    ? "automatic checks are off"
    : lastRefresh === null
      ? "not checked yet"
      : `checked ${timeAgo(lastRefresh)}${
          lastRefresh + intervalSec > nowSec
            ? ` · next in ${Math.max(1, Math.ceil((lastRefresh + intervalSec - nowSec) / 60))}m`
            : " · check due"
        }`;

  return (
    <div className={`app ${focusDetail && openIssue ? "focus" : ""}`}>
      <Sidebar
        projects={projects}
        favCount={favCount}
        selectedId={selectedId}
        pollEnabled={freshnessProps.pollEnabled}
        pollIntervalMinutes={freshnessProps.pollIntervalMinutes}
        onSelect={(id) => {
          setSelectedId(id);
          setOpenIssue(null);
        }}
        onAdd={() => setShowAdd(true)}
        onRemove={(p) => setDeleteCandidate(p)}
        onRename={(p, name) => void renameProject(p, name)}
        onSetNotifications={(p, ni, nc) =>
          void setProjectNotifications(p, ni, nc)
        }
        onReorder={(ids) => void reorderProjects(ids)}
        onOpenSettings={() => setShowSettings(true)}
        onMarkAllSeen={() =>
          void api
            .markAllSeen()
            .then(() => Promise.all([loadProjects(), loadIssues()]))
        }
      />
      <main className="main">
        <header className="topbar">
          <h2>
            {selectedId === "favorites"
              ? "Favorites"
              : selectedProject
                ? selectedProject.name
                : "All projects"}
          </h2>
          {selectedProject && (
            <span className="dim">
              {selectedProject.kind === "drupal" ? "drupal.org queue" : "git.drupalcode.org work items"}
              {selectedProject.last_error && (
                <span className="error inline"> · last refresh failed</span>
              )}
            </span>
          )}
          <span className="dim staleness">{staleness}</span>
        </header>
        {selectedProject?.last_error && (
          <p className="error banner">{selectedProject.last_error}</p>
        )}
        <IssueTable
          issues={issues}
          openIssueId={openIssue?.id ?? null}
          showProject={selectedId === null || selectedId === "favorites"}
          scopeKey={selectedId}
          mode={mode}
          onMode={setMode}
          scopeIssueCount={
            selectedProject
              ? selectedProject.issue_count
              : projects.reduce((a, p) => a + p.issue_count, 0)
          }
          onOpen={(i) => void openIssueRow(i)}
          onToggleFavorite={(i) => void toggleFavorite(i)}
          refreshing={refreshing}
          progress={refreshProgress}
          onRefresh={() => void refreshAll(false)}
          showLoadMore={typeof selectedId === "number"}
          hasMore={hasMore}
          loadingMore={loadingMore}
          onLoadMore={() => void loadMore()}
        />
      </main>
      {openIssue && (
        <IssueDetail
          issue={openIssue}
          hasToken={!!settings?.gitlab_user}
          gitlabUser={settings?.gitlab_user ?? null}
          hideSystemComments={!!settings?.hide_system_comments}
          focused={focusDetail}
          onToggleFocus={() => setFocusDetail((v) => !v)}
          onOpenSettings={() => setShowSettings(true)}
          onToggleFavorite={(i) => void toggleFavorite(i)}
          onUpdated={() => Promise.all([loadProjects(), loadIssues()])}
          onClose={() => {
            setOpenIssue(null);
            setFocusDetail(false);
          }}
        />
      )}
      {showAdd && (
        <AddProjectModal
          onClose={() => setShowAdd(false)}
          onAdded={async (p) => {
            setShowAdd(false);
            await loadProjects();
            if (!p.last_error) setSelectedId(p.id);
          }}
        />
      )}
      {showSettings && settings && (
        <SettingsModal
          settings={settings}
          onSaved={() => void loadSettings()}
          onClose={() => setShowSettings(false)}
        />
      )}
      {deleteCandidate && (
        <div className="modal-backdrop" onClick={() => setDeleteCandidate(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Stop following “{deleteCandidate.name}”?</h2>
            <p className="hint">
              The project and its {deleteCandidate.issue_count} cached issue
              {deleteCandidate.issue_count === 1 ? "" : "s"} will be removed
              from this app. The project itself on drupal.org /
              git.drupalcode.org is not touched.
            </p>
            <div className="row">
              <button
                className="danger"
                onClick={() => {
                  const p = deleteCandidate;
                  setDeleteCandidate(null);
                  void removeProject(p);
                }}
              >
                Remove project
              </button>
              <button className="ghost" onClick={() => setDeleteCandidate(null)}>
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
