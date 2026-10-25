import React from "react";
import { Button } from "../actions/Button.jsx";

export function Notice({ error = false, children, onDismiss }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)" }}>
      <span style={{ flex: 1, fontSize: "var(--fs-13)", color: error ? "var(--status-error)" : "var(--text-2)" }}>{children}</span>
      {onDismiss && <Button size="sm" onClick={onDismiss} style={{ padding: "2px 8px" }}>×</Button>}
    </div>
  );
}

export function ConfirmBanner({ title, children, actions }) {
  return (
    <div
      style={{
        padding: "var(--pad-confirm)",
        borderRadius: "var(--r-1)",
        border: "1px solid color-mix(in srgb, var(--status-update) 60%, transparent)",
        background: "color-mix(in srgb, var(--status-update) 8%, transparent)",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-16)",
      }}
    >
      <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: "var(--sp-2)" }}>
        <span style={{ fontFamily: "var(--font-display)", fontWeight: 600, fontSize: "var(--fs-14)", color: "var(--status-update)" }}>{title}</span>
        <span style={{ fontSize: "var(--fs-12)", color: "var(--text-2)" }}>{children}</span>
      </div>
      <div style={{ display: "flex", gap: "var(--sp-8)" }}>{actions}</div>
    </div>
  );
}

export function Loader({ lit = 0, count = 3 }) {
  return (
    <div style={{ display: "flex", gap: "var(--sp-6)" }}>
      {Array.from({ length: count }, (_, i) => (
        <span key={i} style={{ width: 6, height: 6, transform: "rotate(45deg)", background: i === lit ? "var(--accent)" : "var(--border-control)" }} />
      ))}
    </div>
  );
}
