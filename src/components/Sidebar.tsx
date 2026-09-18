import { useEffect, useRef, useState } from "react";
import type { Project } from "../api";
import { Icon } from "./Icon";
import logo from "../assets/logo.svg";
import drupalMark from "../assets/drupal.svg";
import gitlabMark from "../assets/gitlab.svg";

export function Sidebar({
  projects,
  favCount,
  selectedId,
  pollEnabled,
  pollIntervalMinutes,
  onSelect,
  onAdd,
  onRemove,
  onRename,
  onSetNotifications,
  onReorder,
  onOpenSettings,
  onMarkAllSeen,
}: {
  projects: Project[];
  favCount: number;
  selectedId: number | "favorites" | null;
  /** For the freshness dot: a project is fresh when its last successful check
   * is younger than one polling interval. */
  pollEnabled: boolean;
  pollIntervalMinutes: number;
  onSelect: (id: number | "favorites" | null) => void;
  onAdd: () => void;
  onRemove: (p: Project) => void;
  onRename: (p: Project, name: string) => void;
  /** Store the per-module notification switches (new issues / new comments). */
  onSetNotifications: (
    p: Project,
    notifyNewIssues: boolean,
    notifyNewComments: boolean,
  ) => void;
  onReorder: (ids: number[]) => void;
  onOpenSettings: () => void;
  onMarkAllSeen: () => void;
}) {
  const totalNew = projects.reduce((a, p) => a + p.new_count, 0);
  const totalChanged = projects.reduce((a, p) => a + p.changed_count, 0);

  // Local display order, synced from the backend and optimistically
  // rearranged while dragging.
  const [order, setOrder] = useState<Project[]>(projects);
  const dragFrom = useRef<number | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editValue, setEditValue] = useState("");
  // Which project's notification popover is open, anchored next to its bell.
  // Fixed positioning keeps it out of the scrolling project list's clip.
  const [notifyPop, setNotifyPop] = useState<{
    id: number;
    top: number;
    left: number;
  } | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  // The per-row ⋯ menu: one control instead of three, so the NEW/UPD counts
  // no longer have to be hidden to make room for the actions.
  const [rowMenu, setRowMenu] = useState<{ id: number; top: number; left: number } | null>(null);
  const rowMenuRef = useRef<HTMLDivElement | null>(null);
  const rowMenuTrigger = useRef<HTMLElement | null>(null);

  useEffect(() => {
    setOrder(projects);
  }, [projects]);

  // Outside click or Escape closes either popover; the bell itself re-anchors it.
  useEffect(() => {
    if (!notifyPop && !rowMenu) return;
    const onDown = (e: PointerEvent) => {
      if (!popRef.current?.contains(e.target as Node)) setNotifyPop(null);
      if (!rowMenuRef.current?.contains(e.target as Node)) setRowMenu(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Capture phase: the popovers consume Escape before the detail pane's
        // own handler, so closing a popover never closes the pane behind it.
        e.stopPropagation();
        e.preventDefault();
        setNotifyPop(null);
        setRowMenu(null);
      }
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [notifyPop, rowMenu]);

  const commitOrder = (next: Project[]) => {
    setOrder(next);
    onReorder(next.map((p) => p.id));
  };

  const onDragOver = (index: number) => {
    const from = dragFrom.current;
    if (from === null || from === index) return;
    const next = [...order];
    const [moved] = next.splice(from, 1);
    next.splice(index, 0, moved);
    dragFrom.current = index;
    setDragging(index);
    setOrder(next);
  };

  const startRename = (p: Project) => {
    setNotifyPop(null);
    setRowMenu(null);
    setEditingId(p.id);
    setEditValue(p.name);
  };

  const saveRename = () => {
    if (editingId === null) return;
    const p = projects.find((x) => x.id === editingId);
    const name = editValue.trim();
    if (p && name && name !== p.name) onRename(p, name);
    setEditingId(null);
  };

  const POP_WIDTH = 228;
  const POP_HEIGHT = 150;
  const MENU_WIDTH = 176;
  const MENU_HEIGHT = 132;

  const openNotifyPop = (p: Project, el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    setNotifyPop({
      id: p.id,
      top: Math.max(8, Math.min(r.top, window.innerHeight - POP_HEIGHT - 8)),
      left: Math.max(
        8,
        Math.min(r.right + 6, window.innerWidth - POP_WIDTH - 8),
      ),
    });
  };

  const openRowMenu = (p: Project, el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    setRowMenu({
      id: p.id,
      top: Math.max(8, Math.min(r.bottom + 4, window.innerHeight - MENU_HEIGHT - 8)),
      left: Math.max(8, Math.min(r.right - MENU_WIDTH, window.innerWidth - MENU_WIDTH - 8)),
    });
    rowMenuTrigger.current = el;
  };

  // Read the project back out of the list so a reload refreshes the checkboxes.
  const notifyProject = notifyPop
    ? (projects.find((x) => x.id === notifyPop.id) ?? null)
    : null;

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <h1>
          <img src={logo} className="logo-mark" alt="Drupal Issue Hub logo" />
          Drupal Issue Hub
        </h1>
      </div>
      <button
        className={`side-item all ${selectedId === null ? "selected" : ""}`}
        onClick={() => onSelect(null)}
        title="All projects (⌘0)"
      >
        <span className="name">All projects</span>
        {(totalNew > 0 || totalChanged > 0) && (
          <span className="badges">
            {totalNew > 0 && <span className="badge new">{totalNew}</span>}
            {totalChanged > 0 && <span className="badge changed">{totalChanged}</span>}
          </span>
        )}
      </button>
      <button
        className={`side-item all ${selectedId === "favorites" ? "selected" : ""}`}
        onClick={() => onSelect("favorites")}
        disabled={favCount === 0}
        title="Favorites (⌘⇧F)"
      >
        <span className="star">
          <Icon name="star" size={12} filled />
        </span>
        <span className="name">Favorites</span>
        {favCount > 0 && (
          <span className="badges">
            <span className="badge fav">{favCount}</span>
          </span>
        )}
      </button>

      <div className="project-list">
        {order.map((p, index) => {
          const editing = editingId === p.id;
          const now = Math.floor(Date.now() / 1000);
          const dot = p.last_error
            ? "err"
            : pollEnabled && p.last_refreshed && now - p.last_refreshed < pollIntervalMinutes * 60
              ? "fresh"
              : "stale";
          const dotTitle = p.last_error
            ? `Last refresh failed: ${p.last_error}`
            : dot === "fresh"
              ? "Checked within the last polling interval"
              : "Not checked recently";
          return (
            <div
              key={p.id}
              className={`side-item project ${selectedId === p.id ? "selected" : ""} ${dragging === index ? "dragging" : ""}`}
              draggable={!editing}
              onDragStart={(e) => {
                setNotifyPop(null);
                dragFrom.current = index;
                setDragging(index);
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", String(p.id));
              }}
              onDragOver={(e) => {
                e.preventDefault();
                e.dataTransfer.dropEffect = "move";
                onDragOver(index);
              }}
              onDragEnd={() => {
                dragFrom.current = null;
                setDragging(null);
              }}
              onDrop={(e) => {
                e.preventDefault();
                dragFrom.current = null;
                setDragging(null);
                commitOrder(order);
              }}
              onClick={() => onSelect(p.id)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (editing) return;
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onSelect(p.id);
                }
              }}
            >
              <span
                className="src"
                role="img"
                aria-label={p.kind === "drupal" ? "drupal.org" : "git.drupalcode.org"}
                title={
                  p.kind === "drupal"
                    ? "drupal.org queue"
                    : "git.drupalcode.org work items"
                }
              >
                <img
                  src={p.kind === "drupal" ? drupalMark : gitlabMark}
                  alt=""
                  width={13}
                  height={13}
                />
              </span>
              <span className={`dot ${dot}`} title={dotTitle} />
              {!editing && !p.notify_new_issues && !p.notify_new_comments && (
                <span
                  className="muted-mark"
                  title="Notifications are off for this project"
                >
                  <Icon name="bellOff" size={12} />
                </span>
              )}
              {editing ? (
                <input
                  className="rename-input"
                  autoFocus
                  value={editValue}
                  onChange={(e) => setEditValue(e.target.value)}
                  onClick={(e) => e.stopPropagation()}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") saveRename();
                    if (e.key === "Escape") setEditingId(null);
                  }}
                  onBlur={saveRename}
                />
              ) : (
                <span className="name" title={index < 9 ? `${p.name} (⌘${index + 1})` : p.name}>
                  {p.name}
                </span>
              )}
              {(p.new_count > 0 || p.changed_count > 0) && (
                <span className="badges">
                  {p.new_count > 0 && <span className="badge new">{p.new_count}</span>}
                  {p.changed_count > 0 && (
                    <span className="badge changed">{p.changed_count}</span>
                  )}
                </span>
              )}
              {!editing && (
                <span className="row-actions" onClick={(e) => e.stopPropagation()}>
                  <button
                    className="action"
                    aria-haspopup="menu"
                    aria-expanded={rowMenu?.id === p.id}
                    title={`Actions for ${p.name}`}
                    onClick={(e) => openRowMenu(p, e.currentTarget)}
                  >
                    <Icon name="more" size={14} />
                  </button>
                </span>
              )}
            </div>
          );
        })}
        {order.length === 0 && (
          <p className="empty">No projects yet. Add one to start watching its issue queue.</p>
        )}
      </div>
      <div className="sidebar-footer">
        <button className="ghost" onClick={onMarkAllSeen}>
          Mark all read
        </button>
        <button className="ghost" onClick={onOpenSettings} title="Settings (⌘,)">
          Settings
        </button>
        <button className="primary" onClick={onAdd}>
          <Icon name="plus" size={13} /> Add project
        </button>
      </div>
      {rowMenu &&
        (() => {
          const p = projects.find((x) => x.id === rowMenu.id);
          if (!p) return null;
          return (
            <div className="row-menu" style={{ top: rowMenu.top, left: rowMenu.left }} ref={rowMenuRef}>
              <button
                onClick={() => {
                  const el = rowMenuTrigger.current;
                  setRowMenu(null);
                  if (el) openNotifyPop(p, el);
                }}
              >
                {p.notify_new_issues || p.notify_new_comments ? (
                  <Icon name="bell" size={13} />
                ) : (
                  <Icon name="bellOff" size={13} />
                )}{" "}
                Notifications…
              </button>
              <button onClick={() => startRename(p)}>
                <Icon name="pencil" size={13} /> Rename
              </button>
              <button
                className="danger-item"
                onClick={() => {
                  setRowMenu(null);
                  onRemove(p);
                }}
              >
                <Icon name="x" size={13} /> Stop following
              </button>
            </div>
          );
        })()}
      {notifyPop && notifyProject && (
        <div
          className="notify-pop"
          ref={popRef}
          style={{ top: notifyPop.top, left: notifyPop.left }}
        >
          <div className="notify-title">{notifyProject.name}</div>
          <label className="check">
            <input
              type="checkbox"
              checked={notifyProject.notify_new_issues}
              onChange={(e) =>
                onSetNotifications(
                  notifyProject,
                  e.target.checked,
                  notifyProject.notify_new_comments,
                )
              }
            />
            Notify on new issues
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={notifyProject.notify_new_comments}
              onChange={(e) =>
                onSetNotifications(
                  notifyProject,
                  notifyProject.notify_new_issues,
                  e.target.checked,
                )
              }
            />
            Notify on new comments
          </label>
          <p className="hint">Applies to the automatic background refresh.</p>
        </div>
      )}
    </aside>
  );
}
