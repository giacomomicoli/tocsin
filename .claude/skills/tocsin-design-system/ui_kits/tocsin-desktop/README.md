# Tocsin Desktop — UI kit

Two-column app shell (1120×700): fixed sidebar (logo, installations, page nav) + content column. Mirrors the iced window (`theme::space::WINDOW`, minimum 1000×620): `crates/tocsin/src/app.rs` for the shell and `app/library.rs` for the toolbar, list and rows.

`support.js` is a vendored runtime for the mock pages, not built from this repository. The fonts load from Google Fonts, so the mock needs a network connection to show them.

- `Library.dc.html` — the Library screen. Tweaks: `theme` (void / light), `showTitleBar`.
- `index.html` — Design System card wrapper.

Click-through: filter chips, search, Check updates / Rescan (busy → "Working…"), Update → Rollback, Disable/Enable, Remove → Restore, Install a missing dependency, Retail / Classic Era switch in the sidebar (each has its own addon list), dismiss the notice. **Get addons** (from `app/discover.rs`): WoWInterface catalog loader on first open, search on Enter/button, one result per addon with a line per site (stale years highlighted), Install → "Replace N folders?" confirm when the zip ships folders you have, Installed badge; CurseForge lines appear only after a key is saved. **Settings** (from `app/settings.rs`): game folders (synced with the sidebar; add/remove), API keys (masked input, saved/not set, Remove, Get a key), Save enabled only when something was typed. Tweak `startScreen` opens on any page.

Data is fictional but shaped after `fixtures/README.md` (Routine + Routine Data, Zygor, Details misnamed folder, Ace3, HandyNotes).
