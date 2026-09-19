import { useEffect, useRef, useState } from "react";
import { api, type Settings } from "../api";

export function SettingsModal({
  settings,
  onSaved,
  onClose,
}: {
  settings: Settings;
  onSaved: () => void;
  onClose: () => void;
}) {
  const [pollEnabled, setPollEnabled] = useState(settings.poll_enabled);
  const [interval, setIntervalMin] = useState(settings.poll_interval_minutes);
  const [hideSystem, setHideSystem] = useState(settings.hide_system_comments);
  const [defaultMode, setDefaultMode] = useState<"attention" | "all">(
    settings.default_mode ?? "attention",
  );
  const [theme, setTheme] = useState<"dark" | "light" | "system">(
    settings.theme ?? "system",
  );
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [user, setUser] = useState<string | null>(settings.gitlab_user);
  // One save model: every control writes as it changes, and the form says so.
  const [saved, setSaved] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const savedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intervalTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Capture: the modal consumes Escape so the pane behind it stays open.
        e.stopPropagation();
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [onClose]);

  useEffect(
    () => () => {
      if (savedTimer.current) clearTimeout(savedTimer.current);
      if (intervalTimer.current) clearTimeout(intervalTimer.current);
    },
    [],
  );

  const flashSaved = () => {
    setSaved(true);
    if (savedTimer.current) clearTimeout(savedTimer.current);
    savedTimer.current = setTimeout(() => setSaved(false), 1500);
  };

  // set_settings stores the whole setting set, so every write sends all of it.
  const commit = async (
    next: {
      pollEnabled: boolean;
      interval: number;
      hideSystem: boolean;
      defaultMode: "attention" | "all";
      theme: "dark" | "light" | "system";
    },
    writeDefaultMode = true,
  ) => {
    setBusy(true);
    setError(null);
    try {
      await api.setSettings(
        next.pollEnabled,
        next.interval,
        next.hideSystem,
        writeDefaultMode ? next.defaultMode : null,
        writeDefaultMode ? next.theme : null,
      );
      flashSaved();
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const togglePoll = (next: boolean) => {
    setPollEnabled(next);
    void commit(
      { pollEnabled: next, interval, hideSystem, defaultMode, theme },
      false,
    );
  };

  const toggleHideSystem = (next: boolean) => {
    setHideSystem(next);
    void commit(
      { pollEnabled, interval, hideSystem: next, defaultMode, theme },
      false,
    );
  };

  const chooseDefaultMode = (next: "attention" | "all") => {
    setDefaultMode(next);
    void commit({ pollEnabled, interval, hideSystem, defaultMode: next, theme });
  };

  const chooseTheme = (next: "dark" | "light" | "system") => {
    setTheme(next);
    void commit({ pollEnabled, interval, hideSystem, defaultMode, theme: next });
  };

  // The number field is the one control worth debouncing: "1" then "15" is one
  // decision, not two writes.
  const changeInterval = (raw: number) => {
    const next = Math.min(1440, Math.max(1, Number.isFinite(raw) ? raw : 1));
    setIntervalMin(next);
    if (intervalTimer.current) clearTimeout(intervalTimer.current);
    intervalTimer.current = setTimeout(
      () =>
        void commit(
          { pollEnabled, interval: next, hideSystem, defaultMode, theme },
          false,
        ),
      600,
    );
  };

  // Closing flushes a pending interval write instead of dropping it.
  const close = () => {
    if (intervalTimer.current) {
      clearTimeout(intervalTimer.current);
      intervalTimer.current = null;
      void commit({ pollEnabled, interval, hideSystem, defaultMode, theme }, false);
    }
    onClose();
  };

  const saveToken = async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const username = await api.saveGitlabToken(token.trim());
      setUser(username);
      setToken("");
      setInfo(`Token stored in the macOS keychain for @${username}.`);
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const removeToken = async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      await api.clearGitlabToken();
      setUser(null);
      setConfirmRemove(false);
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={close}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Settings</h2>

        <h3>Background refresh</h3>
        <div className="row">
          <label className="check">
            <input
              type="checkbox"
              checked={pollEnabled}
              onChange={(e) => togglePoll(e.target.checked)}
            />
            Check projects automatically
          </label>
        </div>
        <div className="row">
          <label>
            Every
            <input
              className="num"
              type="number"
              min={1}
              max={1440}
              value={interval}
              onChange={(e) => changeInterval(Number(e.target.value))}
            />
            minutes
          </label>
        </div>

        <h3>Issue list</h3>
        <div className="row">
          <label>
            New projects open on
            <select
              className="num"
              value={defaultMode}
              onChange={(e) => chooseDefaultMode(e.target.value as "attention" | "all")}
            >
              <option value="attention">Things needing attention</option>
              <option value="all">All issues</option>
            </select>
          </label>
          <span className="hint">
            Applies when you open a project; you can always switch inside the list.
          </span>
        </div>

        <h3>Appearance</h3>
        <div className="row">
          <label>
            Colour scheme
            <select
              className="num"
              value={theme}
              onChange={(e) => chooseTheme(e.target.value as "dark" | "light" | "system")}
            >
              <option value="system">Follow system</option>
              <option value="dark">Dark</option>
              <option value="light">Light</option>
            </select>
          </label>
          <span className="hint">
            System follows your OS appearance; dark and light are fixed.
          </span>
        </div>

        <h3>Comments</h3>
        <div className="row">
          <label className="check">
            <input
              type="checkbox"
              checked={hideSystem}
              onChange={(e) => toggleHideSystem(e.target.checked)}
            />
            Hide status-change notes — drupal.org counts these as comments
          </label>
        </div>

        <h3>git.drupalcode.org account (for editing your issues)</h3>
        {user ? (
          <div>
            <p className="hint">
              Signed in as <strong>@{user}</strong>. Comments and edits are
              performed with a personal access token stored in the macOS
              keychain.
            </p>
            {confirmRemove ? (
              <div className="row">
                <span className="hint">
                  Remove it? Commenting and editing stop working until you add a
                  new one.
                </span>
                <button className="danger" disabled={busy} onClick={() => void removeToken()}>
                  Remove token
                </button>
                <button className="ghost" disabled={busy} onClick={() => setConfirmRemove(false)}>
                  Cancel
                </button>
              </div>
            ) : (
              <button className="danger" disabled={busy} onClick={() => setConfirmRemove(true)}>
                Remove token
              </button>
            )}
          </div>
        ) : (
          <div>
            <p className="hint">
              Create a personal access token on{" "}
              <span className="dim">git.drupalcode.org → Preferences → Access tokens</span>{" "}
              with the <code>api</code> scope to comment on and edit issues of
              your own projects.
            </p>
            <div className="row">
              <input
                type="password"
                placeholder="Personal access token"
                value={token}
                onChange={(e) => setToken(e.target.value)}
              />
              <button className="primary" disabled={busy || !token.trim()} onClick={() => void saveToken()}>
                Validate &amp; save
              </button>
            </div>
          </div>
        )}

        <p className="hint">Changes are saved as you make them.</p>
        {busy && <p className="hint">Working…</p>}
        {error && <p className="error">{error}</p>}
        {info && <p className="info">{info}</p>}

        <div className="row modal-footer">
          {saved && <span className="hint info">Saved</span>}
          <button className="ghost" onClick={close}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
