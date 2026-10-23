import React from "react";

export function SearchInput({ value, onChange, placeholder = "Search…", width = "var(--w-search)" }) {
  const [focus, setFocus] = React.useState(false);
  return (
    <input
      value={value}
      placeholder={placeholder}
      onChange={(e) => onChange && onChange(e.target.value)}
      onFocus={() => setFocus(true)}
      onBlur={() => setFocus(false)}
      style={{
        width,
        boxSizing: "border-box",
        padding: "var(--sp-8)",
        fontFamily: "var(--font-ui)",
        fontSize: "var(--fs-13)",
        color: "var(--text-1)",
        background: "var(--input-bg)",
        border: `1px solid ${focus ? "var(--focus-ring)" : "var(--input-border)"}`,
        borderRadius: "var(--r-2)",
        boxShadow: focus ? "var(--shadow-glow-gold)" : "none",
        outline: "none",
      }}
    />
  );
}
