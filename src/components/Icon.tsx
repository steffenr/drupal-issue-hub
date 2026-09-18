/**
 * One inline SVG icon set, replacing the emoji glyphs the UI leaned on
 * (🔔 🔕 ✎ ✕ ★ 💬 🗑 ↻ ＋ 📄 ⑃). Emoji render from whatever font the OS picks,
 * cannot take a colour or a stroke weight from the theme tokens, and differ in
 * size between hosts; these are stroke-based, inherit `currentColor`, and sit on
 * a 24-unit grid at 1.8px so one weight reads across the whole app.
 *
 * Icons are decorative by default (`aria-hidden`); a control that is only an
 * icon carries its name in `title`, which the surrounding button already sets.
 */
const PATHS: Record<string, string> = {
  star: "M12 2.6l2.98 6.04 6.67.97-4.83 4.7 1.14 6.64L12 17.8l-5.96 3.15 1.14-6.64-4.83-4.7 6.67-.97L12 2.6z",
  bell: "M18 8.5a6 6 0 1 0-12 0c0 6-3 7.5-3 7.5h18s-3-1.5-3-7.5M13.7 20a2 2 0 0 1-3.4 0",
  bellOff:
    "M8.8 3.8A6 6 0 0 1 18 8.5c0 2.3.5 3.9.9 4.9M6.7 6.8C6.2 7.3 6 7.9 6 8.5c0 6-3 7.5-3 7.5h12.5M13.7 20a2 2 0 0 1-3.4 0M2.5 2.5l19 19",
  pencil: "M12 20h9M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z",
  x: "M18 6 6 18M6 6l12 12",
  comment: "M21 15a2 2 0 0 1-2 2H8l-5 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z",
  trash:
    "M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6",
  refresh: "M20.5 12a8.5 8.5 0 1 1-2.8-6.3M20.5 4.5v5h-5",
  plus: "M12 5v14M5 12h14",
  file: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5",
  merge:
    "M7 3.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM7 15.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM19 8a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM7 8.5v7M7 12h7a2.5 2.5 0 0 0 2.5-2.5",
  more: "M5.5 12h.01M12 12h.01M18.5 12h.01",
};

export type IconName = keyof typeof PATHS;

export function Icon({
  name,
  size = 14,
  filled = false,
  className,
}: {
  name: IconName;
  size?: number;
  /** Star only: filled marks a favourite, outline does not. */
  filled?: boolean;
  className?: string;
}) {
  return (
    <svg
      className={`icon${className ? ` ${className}` : ""}`}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
