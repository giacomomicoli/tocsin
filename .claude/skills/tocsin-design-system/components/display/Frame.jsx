import React from "react";

export function Frame({ children, padding = "var(--pad-frame)", style }) {
  return (
    <div
      style={{
        border: "1px solid var(--border-frame)",
        borderRadius: "var(--r-3)",
        background: "var(--surface-frame-gradient)",
        boxShadow: "var(--shadow-frame)",
        padding: 3,
        ...style,
      }}
    >
      <div style={{ border: "1px solid var(--border-frame-inner)", borderRadius: "var(--r-1)", padding }}>{children}</div>
    </div>
  );
}

export function GoldRule({ style }) {
  return <div style={{ height: 1, background: "var(--rule-gold)", ...style }} />;
}
