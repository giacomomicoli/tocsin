import * as React from "react";
/**
 * Gilded frame: the header card and dialogs. Two nested containers (outer gold border + gradient + shadow, inner hairline) — buildable in iced by nesting two containers.
 * @startingPoint section="Display" subtitle="Double-bordered gilded container" viewport="700x200"
 */
export interface FrameProps {
  children?: React.ReactNode;
  /** Inner padding; default 14px 18px. */
  padding?: string;
  style?: React.CSSProperties;
}
export function Frame(props: FrameProps): JSX.Element;
export interface GoldRuleProps { style?: React.CSSProperties }
/** 1px horizontal divider fading out at both ends. */
export function GoldRule(props: GoldRuleProps): JSX.Element;
