import { useState } from "react";
import { api, type Project, type ProjectSuggestion } from "../api";

/** The host each suggestion lives on - the URL itself is noise in a list of
 * names, and stays available as the button's tooltip for copying. */
const hostOf = (s: ProjectSuggestion) =>
  s.kind === "gitlab" ? "git.drupalcode.org" : "drupal.org";

export function AddProjectModal({
  onClose,
  onAdded,
}: {
  onClose: () => void;
  onAdded: (p: Project) => void;
}) {
  const [input, setInput] = useState("");
  const [suggestions, setSuggestions] = useState<ProjectSuggestion[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const doSearch = async () => {
    setError(null);
    setSuggestions([]);
    if (input.trim().length < 2) return;
    setBusy(true);
    try {
      setSuggestions(await api.searchSuggestions(input.trim()));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const addUrl = async (url: string) => {
    setBusy(true);
    setError(null);
    try {
      const p = await api.addProject(url);
      onAdded(p);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Add a project to follow</h2>
        <p className="hint">
          Paste a drupal.org project URL (e.g.{" "}
          <code>https://www.drupal.org/project/paragraphs</code>), a
          git.drupalcode.org URL (e.g.{" "}
          <code>https://git.drupalcode.org/project/tmgmt_deepl/-/work_items</code>
          ), or a machine name.
        </p>
        <div className="row">
          <input
            autoFocus
            placeholder="URL or project machine name"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && input.trim()) void addUrl(input.trim());
              if (e.key === "Escape") onClose();
            }}
          />
          <button className="primary" disabled={busy || !input.trim()} onClick={() => void addUrl(input.trim())}>
            Add
          </button>
        </div>
        <div className="row">
          <button className="ghost" disabled={busy || input.trim().length < 2} onClick={() => void doSearch()}>
            Search drupal.org for “{input.trim() || "…"}”
          </button>
        </div>
        {busy && <p className="hint">Working…</p>}
        {error && <p className="error">{error}</p>}
        {suggestions.length > 0 && (
          <ul className="suggestions">
            {suggestions.map((s) => (
              <li key={s.id}>
                <button title={s.add_url} onClick={() => void addUrl(s.add_url)}>
                  <strong>{s.title}</strong>
                  <span className="dim"> · {hostOf(s)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
