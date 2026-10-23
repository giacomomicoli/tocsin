import React from "react";
import { Badge } from "../display/Badge.jsx";

export function AddonRow({ icon, kind = "addon", child = false, disabled = false, name, meta, notes = [], installed, latest, status = "muted", statusLabel, actions }) {
  const [hover, setHover] = React.useState(false);
  const ghost = kind === "missing";
  const removed = kind === "removed" || disabled;
  const box = {
    display: "flex",
    alignItems: "center",
    gap: "var(--sp-16)",
    padding: child || ghost ? "var(--pad-row-child)" : "var(--pad-row)",
    borderRadius: "var(--r-1)",
    border: "1px solid " + (ghost ? "color-mix(in srgb, var(--status-error) 50%, transparent)" : hover && !removed ? "var(--border-row-hover)" : "var(--border-row)"),
    background: ghost ? "transparent" : removed ? "color-mix(in srgb, var(--surface-row) 40%, transparent)" : child ? "var(--surface-child)" : hover ? "var(--surface-row-hover)" : "var(--surface-row)",
    color: removed ? "var(--text-3)" : "var(--text-1)",
  };
  const row = (
    <div style={box} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}>
      {(child || ghost) && <span style={{ fontSize: "var(--fs-14)", color: "var(--text-3)" }}>└</span>}
      <div style={{ width: child || ghost || kind === "removed" ? 24 : 32, height: child || ghost || kind === "removed" ? 24 : 32, flex: "none", boxSizing: "border-box", border: "1px solid var(--border-frame)", borderRadius: "var(--r-1)", background: "var(--surface-sunken)", display: "flex", alignItems: "center", justifyContent: "center", overflow: "hidden", opacity: ghost || removed ? 0.5 : 1 }}>
        {icon ? <img src={icon} alt="" style={{ width: "100%", height: "100%", objectFit: "cover" }} /> : <span style={{ fontFamily: "var(--font-display)", fontWeight: 700, fontSize: child || ghost ? 11 : 14, color: "var(--text-gold)" }}>{String(name).replace(/[^A-Za-z0-9]/g, "").slice(0, 2)}</span>}
      </div>
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: "var(--sp-2)" }}>
        <span style={{ fontSize: child ? "var(--fs-14)" : "var(--fs-15)", fontWeight: 500 }}>{name}</span>
        <span style={{ fontSize: "var(--fs-12)", color: ghost ? "var(--status-error)" : "var(--text-2)" }}>{meta}</span>
        {notes.map((n, i) => (
          <span key={i} style={{ fontSize: "var(--fs-12)", color: n.tone === "error" ? "var(--status-error)" : n.tone === "warn" ? "var(--status-update)" : "var(--text-3)" }}>{n.text}</span>
        ))}
      </div>
      {kind === "addon" && (
        <div style={{ width: "var(--w-versions)", display: "flex", flexDirection: "column", alignItems: "flex-end", gap: "var(--sp-2)", fontFamily: "var(--font-mono)" }}>
          <span style={{ fontSize: "var(--fs-13)" }}>{installed || "—"}</span>
          <span style={{ fontSize: "var(--fs-12)", color: "var(--text-2)" }}>{latest || "—"}</span>
        </div>
      )}
      {kind !== "removed" && (
        <div style={{ width: "var(--w-badge-col)", display: "flex", justifyContent: "flex-end" }}>
          <Badge status={ghost ? "error" : status}>{ghost ? "Missing" : statusLabel}</Badge>
        </div>
      )}
      <div style={{ display: "flex", gap: "var(--sp-8)" }}>{actions}</div>
    </div>
  );
  return child || ghost ? <div style={{ paddingLeft: "var(--indent-child)" }}>{row}</div> : row;
}
