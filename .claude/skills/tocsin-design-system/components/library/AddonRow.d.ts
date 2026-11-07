import * as React from "react";
/**
 * One line of the library: family head, nested member, ghost row for a missing dependency, a removed family awaiting Restore, or a folder holding addons one level too deep (wrapped).
 * @startingPoint section="Library" subtitle="Addon row: head, child, missing, removed, wrapped" viewport="700x320"
 */
export interface AddonRowProps {
  /** Addon icon URL; 32px for heads, 24px otherwise. Falls back to a 2-letter monogram. */
  icon?: string;
  kind?: "addon" | "missing" | "removed" | "wrapped";
  /** Family member: indented 24px with a └ glyph, flatter fill. */
  child?: boolean;
  disabled?: boolean;
  name: string;
  /** "Folder · Source", "required by X · not installed", "removed · v kept until the next change", "8 addons one level too deep · the game does not load them". */
  meta: string;
  notes?: { text: string; tone?: "error" | "warn" | "muted" }[];
  installed?: string;
  /** Latest version, optionally "v · Provider" when a fallback answered. */
  latest?: string;
  status?: 'update' | 'ok' | 'ahead' | 'manual' | 'error' | 'muted';
  statusLabel?: string;
  /** Buttons, in order: Open, Update, Rollback, Disable/Enable, Remove. */
  actions?: React.ReactNode;
}
export function AddonRow(props: AddonRowProps): JSX.Element;
