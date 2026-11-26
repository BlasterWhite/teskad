import type { ReactNode } from "react";

/**
 * The one state marker of Teskad Mobile (brand pack, "États des agents et des
 * tâches"): a shape and a colour for each state, so that colour never carries
 * the meaning alone. The paths are the pack's icons/status/*.svg, inlined so
 * that their inner symbol can use the theme's `--tk-bg` and their colour the
 * text's (`currentColor`, set by `.status-icon--*` in styles.css).
 *
 * Running is an arc that turns once every 1.4 s; it stands still under
 * `prefers-reduced-motion`. An icon is decoration: the word beside it, or the
 * `label`, says the state.
 */
export type Status = "running" | "completed" | "failed" | "active" | "inactive";

const SHAPES: Record<Status, ReactNode> = {
  running: (
    <>
      <circle cx="8" cy="8" r="6" stroke="currentColor" strokeOpacity="0.25" strokeWidth="2" />
      <path d="M8 2a6 6 0 0 1 6 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </>
  ),
  completed: (
    <>
      <circle cx="8" cy="8" r="7" fill="currentColor" />
      <path
        d="M5 8.2 7.2 10.3 11 6"
        stroke="var(--tk-bg)"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </>
  ),
  failed: (
    <>
      <circle cx="8" cy="8" r="7" fill="currentColor" />
      <path d="M5.5 5.5l5 5m0-5l-5 5" stroke="var(--tk-bg)" strokeWidth="1.8" strokeLinecap="round" />
    </>
  ),
  active: <circle cx="8" cy="8" r="4.5" fill="currentColor" />,
  inactive: <circle cx="8" cy="8" r="4.5" stroke="currentColor" strokeWidth="1.6" />,
};

export function StatusIcon({ status, label }: { status: Status; label?: string }) {
  return (
    <span className={`status-icon status-icon--${status}`} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        {SHAPES[status]}
      </svg>
    </span>
  );
}
