import * as React from "react";
/**
 * Status badge for an addon row. One status per UpdateStatus label.
 * @startingPoint section="Display" subtitle="Status badges, colored by UpdateStatus" viewport="700x200"
 */
export interface BadgeProps {
  /** update = Update available, ok = Up to date, ahead = Ahead of source, manual = Check manually / Version unknown, error = Error / Missing, muted = Disabled / Not checked / No source / Ignored. */
  status?: 'update' | 'ok' | 'ahead' | 'manual' | 'error' | 'muted';
  children?: React.ReactNode;
}
export function Badge(props: BadgeProps): JSX.Element;
