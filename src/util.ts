export function timeAgo(ts: number): string {
  if (!ts) return "";
  const seconds = Math.floor(Date.now() / 1000 - ts);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.floor(months / 12)}y ago`;
}

export function dateStr(ts: number): string {
  if (!ts) return "";
  return new Date(ts * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function statusClass(status: string): string {
  const s = status.toLowerCase();
  if (s.startsWith("closed")) return "st-closed";
  if (s === "fixed") return "st-fixed";
  if (s.includes("needs work")) return "st-needs-work";
  if (s.includes("needs review")) return "st-needs-review";
  if (s === "rtbc") return "st-rtbc";
  if (s.includes("postponed")) return "st-postponed";
  if (s === "active" || s === "open") return "st-active";
  return "st-other";
}
