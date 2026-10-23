import React from "react";

const SIZES = {
  sm: { padding: "var(--pad-btn-sm)", fontSize: "var(--fs-12)" },
  md: { padding: "var(--pad-btn-md)", fontSize: "var(--fs-13)" },
  chip: { padding: "var(--pad-chip)", fontSize: "var(--fs-13)" },
};

function look(variant, state) {
  if (variant === "primary") {
    const bg = state === "hover" ? "var(--btn-primary-bg-hover)" : state === "press" ? "var(--btn-primary-bg-pressed)" : "var(--btn-primary-bg)";
    return { background: bg, borderColor: "var(--btn-primary-border)", color: "var(--btn-primary-text)" };
  }
  if (variant === "selected") {
    return { background: "var(--btn-selected-bg)", borderColor: "var(--btn-selected-border)", color: "var(--btn-selected-text)" };
  }
  if (variant === "danger") {
    return {
      background: state === "idle" ? "transparent" : "color-mix(in srgb, var(--status-error) 16%, transparent)",
      borderColor: "color-mix(in srgb, var(--status-error) 55%, transparent)",
      color: "var(--status-error)",
    };
  }
  return { background: state === "idle" ? "transparent" : "var(--btn-ghost-bg-hover)", borderColor: "var(--btn-ghost-border)", color: "var(--btn-ghost-text)" };
}

export function Button({ variant = "secondary", size = "sm", disabled = false, onClick, children, style }) {
  const [state, setState] = React.useState("idle");
  const l = look(variant, disabled ? "idle" : state);
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      onMouseEnter={() => setState("hover")}
      onMouseLeave={() => setState("idle")}
      onMouseDown={() => setState("press")}
      onMouseUp={() => setState("hover")}
      style={{
        ...SIZES[size],
        ...l,
        fontFamily: variant === "primary" || variant === "selected" ? "var(--font-display)" : "var(--font-ui)",
        fontWeight: variant === "primary" || variant === "selected" ? 600 : 500,
        lineHeight: "var(--lh-ui)",
        borderWidth: 1,
        borderStyle: "solid",
        borderRadius: "var(--r-2)",
        cursor: disabled ? "default" : "pointer",
        opacity: disabled ? 0.4 : 1,
        whiteSpace: "nowrap",
        ...style,
      }}
    >
      {children}
    </button>
  );
}
