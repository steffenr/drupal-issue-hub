import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { api, type GitlabLabel, type Issue } from "../api";
import { Icon } from "./Icon";

/**
 * drupal.org's own taxonomy on git.drupalcode.org, expressed as GitLab scoped
 * labels — `key::value` with a double colon, which is what the projects really
 * carry. Each namespace holds exactly one value per issue, so picking one
 * replaces the value that was set before instead of stacking two.
 */
const SCOPED: Record<string, string[]> = {
  category: [
    "category::bug",
    "category::feature",
    "category::plan",
    "category::support",
    "category::task",
  ],
  priority: [
    "priority::critical",
    "priority::major",
    "priority::minor",
    "priority::normal",
  ],
  state: [
    "state::accepted",
    "state::closed",
    "state::fixed",
    "state::needsReview",
    "state::needsWork",
    "state::postponed",
    "state::rtbc",
    "state::toBePorted",
  ],
  why: [
    "why::cannotReproduce",
    "why::duplicate",
    "why::needsInfo",
    "why::outdated",
    "why::wontFix",
    "why::worksAsDesigned",
  ],
};

function namespaceOf(label: string): string | null {
  const i = label.indexOf("::");
  if (i < 0) return null;
  const ns = label.slice(0, i).toLowerCase();
  return ns in SCOPED ? ns : null;
}

/** `state::needsReview` reads as "Needs review" — the drupal.org wording — while
 * anything outside the known namespaces is shown as the project spells it. */
export function prettyLabel(label: string): string {
  const i = label.indexOf("::");
  if (i < 0) return label;
  // "::" is two characters: slice(i + 1) would keep the second colon and the
  // pill would read ":normal" instead of "Normal".
  const value = label.slice(i + 2).trim();
  const spaced = value.replace(/([A-Z])/g, " $1").trim();
  const out = spaced.charAt(0).toUpperCase() + spaced.slice(1).toLowerCase();
  return out === "Rtbc" ? "RTBC" : out;
}

/** Add a label, keeping one value per drupal.org namespace. GitLab enforces
 * this itself for `key::value` labels, but only on Premium and above, so the
 * picker cannot rely on the server to drop the previous value. */
function applyNamespace(labels: string[], name: string): string[] {
  const ns = namespaceOf(name);
  const kept = ns ? labels.filter((l) => namespaceOf(l) !== ns) : labels;
  return [...kept, name];
}

const has = (labels: string[], name: string) =>
  labels.some((l) => l.toLowerCase() === name.toLowerCase());

function diff(next: string[], saved: string[]): { add: string[]; remove: string[] } {
  const nextLower = new Set(next.map((l) => l.toLowerCase()));
  const savedLower = new Set(saved.map((l) => l.toLowerCase()));
  return {
    add: next.filter((l) => !savedLower.has(l.toLowerCase())),
    remove: saved.filter((l) => !nextLower.has(l.toLowerCase())),
  };
}

function dot(color: string): CSSProperties {
  return { background: color || "var(--border)" };
}

/** Namespaces the issue header already renders as a status / category / version
 * pill. Repeating them as labels would show the same fact in two shapes, so the
 * label row leaves them to the pills above it. Both the scoped
 * (`state::needsWork`) and the migrated (`Status: Needs review`) spelling are
 * recognised, because the backend derives the pills from either. */
const PILL_NAMESPACES = new Set(["state", "status", "category", "version"]);

function shownByPills(label: string): boolean {
  const scoped = label.indexOf("::");
  const at = scoped >= 0 ? scoped : label.indexOf(":");
  if (at < 0) return false;
  return PILL_NAMESPACES.has(label.slice(0, at).trim().toLowerCase());
}

/** Beyond this the header starts pushing the description off screen; the rest is
 * one click away, and the picker always lists everything. */
const PILL_CAP = 6;

/** Pill text: drupal.org's own namespaces read as the site words them
 * (`priority::major` → "Major"), everything else keeps the project's spelling so
 * `component::forms` cannot be mistaken for `tags::forms`. */
function pillText(label: string): string {
  return namespaceOf(label) ? prettyLabel(label) : label;
}

export function LabelEditor({
  issue,
  projectId,
  editable,
  onSaved,
}: {
  issue: Issue;
  projectId: number;
  /** Labels belong to the data even when they cannot be changed: read-only rows
   * show their pills, and only a writable one gets the trigger. */
  editable: boolean;
  onSaved: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [next, setNext] = useState<string[]>(issue.labels);
  const [palette, setPalette] = useState<GitlabLabel[] | null>(null);
  const [paletteError, setPaletteError] = useState<string | null>(null);
  const [loadingPalette, setLoadingPalette] = useState(false);
  const [text, setText] = useState("");
  const [suggest, setSuggest] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Staged edits must survive a background refresh of the issue row, and a
  // failed palette read must not be retried on every keystroke.
  const dirty = useRef(false);
  const attempted = useRef(false);

  const signature = issue.labels.join("\n");
  useEffect(() => {
    if (dirty.current) return;
    setNext(issue.labels);
  }, [issue.id, signature, issue.labels]);

  // Another issue is not this one's draft.
  useEffect(() => {
    setOpen(false);
    setExpanded(false);
    setText("");
    setError(null);
    dirty.current = false;
  }, [issue.id]);

  // Escape closes the picker - in the capture phase, so the picker consumes
  // the key before the detail pane's own Escape handler.
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        e.preventDefault();
        setOpen(false);
      }
    };
    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [open]);

  const loadPalette = async (refresh: boolean) => {
    setPaletteError(null);
    setLoadingPalette(true);
    try {
      setPalette(await api.projectLabels(projectId, refresh));
    } catch (e) {
      setPaletteError(String(e));
      setPalette([]);
    } finally {
      setLoadingPalette(false);
    }
  };

  // The project's own labels are only needed once the editor is opened, and
  // only cost one cached request per project.
  useEffect(() => {
    if (!open || attempted.current || palette !== null) return;
    attempted.current = true;
    void loadPalette(false);
  }, [open, palette, projectId]);

  const suggestions = useMemo(() => {
    const q = text.trim().toLowerCase();
    if (!q) return [];
    const pool = new Map<string, GitlabLabel>();
    for (const l of palette ?? []) pool.set(l.name.toLowerCase(), l);
    for (const names of Object.values(SCOPED)) {
      for (const n of names) {
        if (!pool.has(n.toLowerCase())) {
          pool.set(n.toLowerCase(), { name: n, color: "", description: "" });
        }
      }
    }
    return [...pool.values()]
      .filter((l) => l.name.toLowerCase().includes(q) && !has(next, l.name))
      .sort((a, b) => a.name.localeCompare(b.name))
      .slice(0, 12);
  }, [text, palette, next]);

  // A name the project does not have yet is created there by GitLab when it is
  // assigned, so warn before it lands in the shared tracker.
  const unknown =
    text.trim().length > 0 &&
    palette !== null &&
    !palette.some((l) => l.name.toLowerCase() === text.trim().toLowerCase());

  const colorOf = (name: string) =>
    (palette ?? []).find((l) => l.name.toLowerCase() === name.toLowerCase())?.color ?? "";

  const dirtyNow = next.join("\u0000") !== issue.labels.join("\u0000");

  const toggle = (name: string) => {
    dirty.current = true;
    setNext((prev) =>
      has(prev, name)
        ? prev.filter((l) => l.toLowerCase() !== name.toLowerCase())
        : applyNamespace(prev, name),
    );
    setError(null);
  };

  const addLabel = (name: string) => {
    const trimmed = name.trim();
    if (!trimmed || has(next, trimmed)) return;
    dirty.current = true;
    setNext((prev) => applyNamespace(prev, trimmed));
    setText("");
    setSuggest(false);
  };

  const cancel = () => {
    dirty.current = false;
    setNext(issue.labels);
    setText("");
    setError(null);
  };

  const save = async () => {
    const { add, remove } = diff(next, issue.labels);
    if (!add.length && !remove.length) return;
    setBusy(true);
    setError(null);
    try {
      await api.setIssueLabels(issue.id, add, remove);
      dirty.current = false;
      // Done is done: back to the collapsed row of pills, with no half-typed
      // name waiting on a reopen. A failed save stays open so it can be retried.
      setOpen(false);
      setText("");
      setSuggest(false);
      setExpanded(false);
      // A label that did not exist before only shows up in the project's list
      // once it has been re-read; without this it would stay colourless and be
      // flagged as new on every keystroke.
      if (add.length > 0) void loadPalette(true);
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const otherLabels = next.filter((l) => namespaceOf(l) === null);
  // The pills above already carry state / category / version, in drupal.org
  // wording, so the label row shows what those do not.
  const pillLabels = next.filter((l) => !shownByPills(l));
  const shown = expanded ? pillLabels : pillLabels.slice(0, PILL_CAP);
  const overflow = pillLabels.length - shown.length;

  return (
    <div className="labels-meta">
      {(pillLabels.length > 0 || editable) && (
        <p className="meta label-row">
          {shown.map((n) => (
            <span
              key={n}
              className="label-pill"
              title={n}
              style={colorOf(n) ? { borderColor: `${colorOf(n)}66` } : undefined}
            >
              {/* The colour is only known once the project's labels have been
                  read; a grey dot next to every pill would be noise. */}
              {colorOf(n) && <span className="label-pill-dot" style={dot(colorOf(n))} />}
              {pillText(n)}
            </span>
          ))}
          {overflow > 0 && (
            <button className="ghost small" onClick={() => setExpanded(true)}>
              +{overflow} more
            </button>
          )}
          {expanded && pillLabels.length > PILL_CAP && (
            <button className="ghost small" onClick={() => setExpanded(false)}>
              fewer
            </button>
          )}
          {editable && (
            <button
              className="label-trigger"
              aria-expanded={open}
              title="Add or remove labels"
              onClick={() => setOpen((o) => !o)}
            >
              {open ? (
                <>
                  <Icon name="x" size={12} /> close
                </>
              ) : (
                <>
                  <Icon name="plus" size={12} /> label
                </>
              )}
            </button>
          )}
          {dirtyNow && <span className="hint label-unsaved">unsaved</span>}
        </p>
      )}

      {editable && open && (
        <div className="label-panel">
          {Object.entries(SCOPED).map(([ns, names]) => (
            <div className="label-group" key={ns}>
              <span className="label-group-name">{ns}</span>
              <div className="label-chips">
                {names.map((n) => (
                  <button
                    key={n}
                    className={`label-chip ${has(next, n) ? "on" : ""}`}
                    title={n}
                    disabled={busy}
                    onClick={() => toggle(n)}
                  >
                    {prettyLabel(n)}
                  </button>
                ))}
              </div>
            </div>
          ))}

          {otherLabels.length > 0 && (
            <div className="label-group">
              <span className="label-group-name">other</span>
              <div className="label-chips">
                {otherLabels.map((n) => (
                  <button
                    key={n}
                    className="label-chip on"
                    title={`Remove ${n}`}
                    disabled={busy}
                    onClick={() => toggle(n)}
                  >
                    <span className="label-dot" style={dot(colorOf(n))} />
                    {namespaceOf(n) ? prettyLabel(n) : n} ×
                  </button>
                ))}
              </div>
            </div>
          )}

          <div className="label-add">
            <input
              placeholder="Add a label — existing or new…"
              value={text}
              disabled={busy}
              onChange={(e) => {
                setText(e.target.value);
                setSuggest(true);
              }}
              onFocus={() => setSuggest(true)}
              onBlur={() => window.setTimeout(() => setSuggest(false), 150)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addLabel(text.trim() || suggestions[0]?.name || "");
                } else if (e.key === "Escape") {
                  setSuggest(false);
                }
              }}
            />
            {suggest && suggestions.length > 0 && (
              <ul className="label-suggest">
                {suggestions.map((l) => (
                  <li key={l.name}>
                    <button
                      // Keep focus on the input until the click has landed.
                      onMouseDown={(e) => e.preventDefault()}
                      title={l.description || l.name}
                      onClick={() => addLabel(l.name)}
                    >
                      <span className="label-dot" style={dot(l.color)} />
                      {l.name}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>

          {unknown && (
            <p className="hint">
              “{text.trim()}” is not a label of this project yet — saving creates
              it there for everyone.
            </p>
          )}

          <div className="row">
            <button
              className="primary"
              disabled={busy || !dirtyNow}
              onClick={() => void save()}
            >
              Save labels
            </button>
            <button className="ghost" disabled={busy} onClick={cancel}>
              Cancel
            </button>
            <button
              className="ghost small"
              disabled={busy || loadingPalette}
              title="Re-read this project's labels from git.drupalcode.org"
              onClick={() => {
                attempted.current = true;
                void loadPalette(true);
              }}
            >
              {loadingPalette ? "Refreshing…" : "Refresh list"}
            </button>
          </div>

          {paletteError && (
            <p className="hint">
              Could not read this project's labels: {paletteError} The scoped
              choices above still work.
            </p>
          )}
          {error && <p className="error">{error}</p>}
        </div>
      )}
    </div>
  );
}
