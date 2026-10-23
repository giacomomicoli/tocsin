import * as React from "react";
/**
 * Tocsin button. Maps 1:1 to the iced styles in theme.rs: primary (crimson + gold), secondary (ghost), danger (outlined red), selected (active nav/filter chip).
 * @startingPoint section="Actions" subtitle="Crimson-and-gold primary, ghost, danger, selected" viewport="700x200"
 */
export interface ButtonProps {
  /** primary = the one main action per area; selected = active nav/filter chip. */
  variant?: "primary" | "secondary" | "danger" | "selected";
  /** sm 5/10 @12px (row actions), md 8/14 @13px (header actions), chip 6/12 @13px (nav, filters). */
  size?: "sm" | "md" | "chip";
  disabled?: boolean;
  onClick?: () => void;
  children?: React.ReactNode;
  style?: React.CSSProperties;
}
export function Button(props: ButtonProps): JSX.Element;
