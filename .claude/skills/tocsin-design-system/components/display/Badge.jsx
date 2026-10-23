import React from "react";

export function Badge({ status = "muted", children }) {
  const c = `var(--status-${status})`;
  return (
    <span
      style={{
        display: "inline-block",
        fontFamily: "var(--font-display)",
        fontWeight: 600,
        fontSize: "var(--fs-11)",
        lineHeight: "var(--lh-ui)",
        padding: "var(--pad-badge)",
        borderRadius: "var(--r-1)",
        color: c,
        border: `1px solid color-mix(in srgb, ${c} 50%, transparent)`,
        background: `color-mix(in srgb, ${c} 12%, transparent)`,
        whiteSpace: "nowrap",
        textTransform: "uppercase",
      }}
    >
      {children}
    </span>
  );
}
