export type ProjectKind = "drupal" | "gitlab";

export interface Project {
  id: number;
  kind: ProjectKind;
  key: string;
  name: string;
  url: string;
  last_refreshed: number | null;
  last_error: string | null;
  new_count: number;
  changed_count: number;
  issue_count: number;
  notify_new_issues: boolean;
  notify_new_comments: boolean;
}

export interface Issue {
  id: number;
  project_id: number;
  project_name: string;
  source: ProjectKind;
  ext_id: string;
  title: string;
  url: string;
  status_label: string;
  category_label: string;
  version: string;
  author: string;
  created_at: number;
  changed_at: number;
  body: string;
  comment_count: number;
  labels: string[];
  favorite: boolean;
  is_new: boolean;
  has_changes: boolean;
}

export interface Comment {
  ext_id: string;
  author: string;
  body: string;
  created_at: number;
  system: boolean;
  is_new: boolean;
}

export interface AssetLink {
  name: string;
  url: string;
  kind: "patch" | "mr";
}

/** One label a GitLab project offers, as cached from its labels API. */
export interface GitlabLabel {
  name: string;
  color: string;
  description: string;
}

export interface RefreshResult {
  project_id: number;
  project_name: string;
  ok: boolean;
  error: string | null;
  new_issues: number;
  changed_issues: number;
  /** Comments added to issues this project already had (count deltas). */
  /** Comments added to issues this project already had (count deltas). */
  new_comments: number;
  /** Newest issue this refresh found news in, for following a notification. */
  target_issue_id: number | null;
  /** The project's switches, so the caller decides what is reportable. */
  notify_new_issues: boolean;
  notify_new_comments: boolean;
}

export interface Settings {
  poll_enabled: boolean;
  poll_interval_minutes: number;
  hide_system_comments: boolean;
  /** Table filter a freshly opened project starts in: "attention" or "all". */
  default_mode: "attention" | "all";
  gitlab_user: string | null;
}

export interface ProjectSuggestion {
  id: string;
  kind: string;
  title: string;
  add_url: string;
}

import { invoke } from "@tauri-apps/api/core";

export const api = {
  addProject: (url: string) => invoke<Project>("add_project", { url }),
  searchSuggestions: (query: string) =>
    invoke<ProjectSuggestion[]>("search_suggestions", { query }),
  listProjects: () => invoke<Project[]>("list_projects"),
  listIssues: (projectId: number, unseenOnly: boolean, favoritesOnly: boolean) =>
    invoke<Issue[]>("list_issues", { projectId, unseenOnly, favoritesOnly }),
  getIssue: (issueId: number) => invoke<Issue | null>("get_issue", { issueId }),
  toggleFavorite: (issueId: number) => invoke<boolean>("toggle_favorite", { issueId }),
  favoritesCount: () => invoke<number>("favorites_count"),
  refreshProject: (projectId: number) =>
    invoke<RefreshResult>("refresh_project", { projectId }),
  loadMoreIssues: (projectId: number) =>
    invoke<{ loaded: number; hasMore: boolean }>("load_more_issues", { projectId }),
  refreshAll: () => invoke<RefreshResult[]>("refresh_all"),
  removeProject: (projectId: number) =>
    invoke<void>("remove_project", { projectId }),
  renameProject: (projectId: number, name: string) =>
    invoke<void>("rename_project", { projectId, name }),
  setProjectNotifications: (
    projectId: number,
    notifyNewIssues: boolean,
    notifyNewComments: boolean,
  ) =>
    invoke<void>("set_project_notifications", {
      projectId,
      notifyNewIssues,
      notifyNewComments,
    }),
  reorderProjects: (projectIds: number[]) =>
    invoke<void>("reorder_projects", { projectIds }),
  markIssueSeen: (issueId: number) => invoke<void>("mark_issue_seen", { issueId }),
  markProjectSeen: (projectId: number) =>
    invoke<void>("mark_project_seen", { projectId }),
  markAllSeen: () => invoke<void>("mark_all_seen"),
  getComments: (issueId: number) => invoke<Comment[]>("get_comments", { issueId }),
  getIssueAssets: (issueId: number) => invoke<AssetLink[]>("get_issue_assets", { issueId }),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (
    pollEnabled: boolean,
    pollIntervalMinutes: number,
    hideSystemComments: boolean,
    defaultMode?: "attention" | "all" | null,
  ) =>
    invoke<void>("set_settings", {
      pollEnabled,
      pollIntervalMinutes,
      hideSystemComments,
      defaultMode,
    }),
  saveGitlabToken: (token: string) =>
    invoke<string>("save_gitlab_token", { token }),
  clearGitlabToken: () => invoke<void>("clear_gitlab_token"),
  addIssueComment: (issueId: number, body: string) =>
    invoke<void>("add_issue_comment", { issueId, body }),
  editIssueComment: (issueId: number, extId: string, body: string) =>
    invoke<void>("edit_issue_comment", { issueId, extId, body }),
  deleteIssueComment: (issueId: number, extId: string) =>
    invoke<void>("delete_issue_comment", { issueId, extId }),
  updateIssue: (issueId: number, title: string, description: string) =>
    invoke<void>("update_issue", { issueId, title, description }),
  setIssueState: (issueId: number, close: boolean) =>
    invoke<void>("set_issue_state", { issueId, close }),
  setIssueLabels: (issueId: number, add: string[], remove: string[]) =>
    invoke<void>("set_issue_labels", { issueId, add, remove }),
  projectLabels: (projectId: number, refresh: boolean) =>
    invoke<GitlabLabel[]>("get_project_labels", { projectId, refresh }),
};
