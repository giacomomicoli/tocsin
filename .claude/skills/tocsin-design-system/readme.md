# tocsin — Design System

Visual language for **Tocsin**, a small native World of Warcraft addon update checker written in Rust with **iced** (Windows + Linux). The look is a fantasy theme in the spirit of *WoW: Midnight*: void-dark surfaces, Silvermoon gold trim, crimson-and-gold primary buttons, with a light "Sunwell" parchment theme of equal standing.

## Where it lives in the code
The system is implemented, and the app is the reference: `crates/tocsin/src/theme.rs` (tokens of both themes, fonts, logo bytes, every container / button / text / input / scrollbar style), `crates/tocsin/src/app/widgets.rs` (page head, gold rule, gilded frame, icon slot, badge, buttons, chips), and the page views in `app.rs` (shell: sidebar, nav, footer), `app/library.rs`, `app/discover.rs`, `app/settings.rs`. Status labels come from `crates/tocsin-core/src/checker.rs` (`UpdateStatus::label()`). `rust/theme.rs` here is the earlier copy of that file the app was built from; when the two differ, the crate's file wins.

## Product context
One window, three pages: **Library** (installed addons, grouped by family), **Get addons** (search CurseForge, WoWInterface, GitHub, Tukui), **Settings** (game folders, API keys, theme). No account, no telemetry, no web view. Window 1120×700 (`theme::space::WINDOW`), minimum 1000×620 (`main.rs`).

## The iced rule (read first)
Every style here must map to an iced `container::Style` / `button::Style`:
- one background (solid color **or** linear gradient), one border (color, width, radius), one shadow (offset, blur, color), a text color;
- no letter-spacing, no text-shadow, no blur/backdrop, no transforms on containers, no tweened transitions;
- ornament comes from **nesting** containers (the gilded frame is two), gradient fills, colored glow shadows, and unicode glyphs (`└ × ◆ ─`).
If a mock needs something outside this list, it isn't part of the system.

## CONTENT FUNDAMENTALS
Copy comes straight from the codebase; keep its voice.
- **Plain, short, sentence case.** "Check updates", "Get addons", "Needs attention 3", "No addons found. Check the game folder in Settings."
- **Second person, implied.** Rarely says "you"; never "we". README voice is dry and a little wry ("it reads `.toc` files. Sorry.").
- **Facts over reassurance.** "required by Routine · not installed", "removed · 11.1.2 kept until the next change", "CurseForge offers no direct download — use Open".
- **Middle dot ` · ` separates facts**; em dash for a consequence. Ellipsis on progress labels: "Checking…", "Working…", "Installing…", "Search…".
- **Errors are for the user:** short, no URLs, no internals.
- **Status labels are fixed** (`UpdateStatus::label()`): Update available, Up to date, Ahead of source, Version unknown, Check manually, No source, Ignored, Error — plus Missing, Disabled, Not checked. Badges render them uppercase.
- The fantasy theme is **visual only** — no lore-speak in UI copy ("Summon updates" ✗).
- No emoji.

## VISUAL FOUNDATIONS
- **Color.** Void ramp (`--void-*`) for surfaces; gold (`--gold-*`) for frame trim, titles, selected state, focus; crimson gradient for the single primary action. Light theme swaps to parchment/ink with deeper golds. Status hues are borrowed from WoW item quality (legendary orange = update, uncommon green = ok, epic violet = ahead, rare blue = manual, red = error), tuned per theme for contrast.
- **Type.** Cinzel (display: wordmark, primary/selected button labels, badges, section heads) + Alegreya Sans (all UI text) + JetBrains Mono (versions, folders, paths). Sizes are the iced ones: 24 / 18 / 15 / 14 / 13 / 12 / 11.
- **Spacing.** Pixel steps 2–24; compact. Content padding 20, page gap 14, row gap 8, row padding 12/14 (child 10/14, indent 24), gilded frame 3 + 16/18. The full list is `tokens/spacing.css` = `theme::space`.
- **Backgrounds.** Vertical linear gradient on the app (void → deeper void; parchment → sand). No imagery, no textures, no patterns — iced would need image widgets and we have no licensed art.
- **Frames.** The gilded frame: outer 1px gold-600, r4, gradient fill, drop shadow; 3px inset; inner 1px gold hairline at 16%, r2. In the app it holds the Settings sections only (Game Folders, API Keys, Theme). A 1px horizontal gold gradient rule divides sections (under every page head, in the sidebar).
- **Rows / cards.** Flat `--surface-row`, 1px `--border-row`, r2. Children: `--surface-child`, indented with `└`. Missing dependency: transparent with red-tinted border. Disabled/removed: 40% fill, `--text-3`.
- **Corners.** Near-square: 2 (badge, row), 3 (button, input), 4 (frame). No pills.
- **Shadows.** One per container. Frame drop shadow; gold glow on focus/selected; no inner shadows.
- **Hover.** Ghost buttons get a 8% gold wash; primary brightens to crimson-400; rows switch border to `--border-row-hover` (gold-700). **Press:** primary gradient inverts. No scale/shrink.
- **Disabled.** 40% opacity.
- **Motion.** None in styles (instant state swaps). The only animation is the three-diamond loader stepping every 250ms.
- **Transparency.** Only for tints (badge fill 12%, border 50%, confirm banner 8%) — never blur.
- **Layout — app shell (every page).** Window 1120×700 (the mock draws a 30px title bar; the app uses the OS one). Two columns:
  - **Sidebar, 240px, never changes between pages.** `--surface-frame-gradient`, separated by a 1px `--border-frame` column (iced: a 1px-wide container — borders are all-sides only). Padding 18/14/14, gap 14. Top→bottom: lockup 52px tall · gold rule · "INSTALLATIONS" label (Cinzel 11, `--text-gold`) · one item per game folder (32px monogram slot, flavor name in Cinzel 13, "build · N addons" 12, truncated path mono 10, update-count tag) · "+ Add game folder" (dashed ghost) · flex spacer · gold rule · page nav (Library, Get addons, Settings) as full-width items, selected = gold `--btn-selected-*`.
  - **Content, fills the rest, padding 20, gap 14.** Only this column changes per page. Always: page head (Cinzel 18 `--text-title` + mono "Flavor build · path" line, page actions right) · gold rule · page body · footer notice line.
  - Selecting an installation re-scopes the content column; the selected install also names the window ("Tocsin — Retail").
  - Library body: filter chips + 240px search, scrolling list; right-aligned fixed columns: versions 120px, badge 150px.
  - Get addons body: full-width search (input padding 10 + primary Search 9/16), catalog/key status lines, optional confirm banner, results list; result row = 32px icon slot · name/by/summary · 400px sources column (provider 96px · facts · Open · Install, or a "Site only" `Tone::Manual` badge when the site has no file for apps, followed by a HelpTip "?" that opens the reason under the line).
  - Settings body: gilded-frame sections (Game Folders, API Keys; section title Cinzel 16), Save bottom-right. Settings is the only page that uses frames in the content column.

## ICONOGRAPHY
The app uses **no icon set**. Every action is a text button, and the only glyphs are unicode: `└` (family member), `×` (dismiss), `…` (progress), `?` (the HelpTip toggle, in Cinzel). The system keeps it that way; `◆` is allowed as an ornament/loader glyph. If icons are ever needed, the candidate is **Lucide** (it ships a TTF icon font usable from iced via `Font::with_name`); nothing uses it today, and adding it means extending the system first. No emoji. Never use Blizzard art, the WoW logo or game icons.

## Logo
The project's own artwork (`assets/logo/`, transparent PNGs, trimmed):
- **Emblem** — `tocsin-emblem.png` (1206×1221) + `-256`, `-64`, `-32`. App / window / taskbar icon. In the app: `theme::EMBLEM_PNG` (the 256 one), decoded with the PNG-only `image` crate and passed to `window::icon::from_rgba` — iced's `from_file_data` needs its every-codec feature.
- **Lockup** — `tocsin-lockup.png` (emblem + "Addon Manager", 1644×712) + `-160h`. In-app logo, 52px tall at the top of the sidebar; no tagline beside it. In the app: `theme::LOCKUP_PNG`, made into an `image::Handle` once in `App::boot` and cloned in `view` (a handle built in `view` is decoded again every frame).
Gold-on-transparent: works on both themes; never place on a busy or gold background, never recolor.

## Addon icons
Every addon row and search result reserves an icon slot before the name: 32×32 for heads and results, 24×24 for family members, missing deps and removed rows. 1px `--border-frame`, r2, `--surface-sunken` fill. It shows the addon's picture when the core found one (CurseForge logo, else WoWInterface thumbnail; `tocsin_core::icons`, decoded to a 64px square), otherwise a two-letter Cinzel monogram in `--text-gold`. Dimmed for missing / disabled / removed: the picture at 50% opacity, the monogram in `--text-3`. The GUI never fetches or decodes a picture itself.

## Fonts — substitution flag
Cinzel, Alegreya Sans and JetBrains Mono are loaded from Google Fonts in the HTML cards (`tokens/fonts.css`). They are stand-ins for WoW's proprietary Friz Quadrata / Morpheus. The app ships the static TTFs (all OFL, licences alongside) in `crates/tocsin/assets/fonts/`, compiled in with `include_bytes!` — never fetched at runtime. Alegreya Sans has no box-drawing glyphs: `└` falls back to another font.

## Index
- `styles.css` — entry point (imports only)
- `tokens/` — `colors.css` (Void default + `[data-theme="light"]`), `typography.css`, `spacing.css` (spacing, paddings, radii, column widths), `effects.css` (shadows, glow), `fonts.css`
- `assets/logo/` — emblem + lockup
- `guidelines/` — foundation specimen cards (Colors, Type, Spacing, Brand)
- `components/`
  - `actions/Button` — primary · secondary · danger · selected; sm / md / chip
  - `display/Badge` — status badge; `display/Frame` + `GoldRule`
  - `forms/SearchInput`
  - `feedback/Notice` + `ConfirmBanner` + `Loader`; `feedback/HelpTip`: a "?" that opens or closes the explanation of a badge
  - `library/AddonRow` — head, child, missing, removed
- `ui_kits/tocsin-desktop/` — the visual reference: all three pages in the two-column shell (`Library.dc.html`, tweak `startScreen` and `theme`)
- `rust/theme.rs` — the tokens and styles as iced 0.14 code, the earlier version (the crate's `theme.rs` has moved on; see `rust/INTEGRATION.md` §9)
- `rust/INTEGRATION.md` — how the system was applied: assets, Cargo features, main.rs, app shell, migration table
- `SKILL.md` — the workflow every UI change follows

Component inventory mirrors what `theme.rs` and `library.rs` define (primary/ghost/danger buttons, badge, card, row variants, confirm, loader dot, text input, notice). **Intentional additions:** `selected` button variant (the code reuses `primary` for active chips; a distinct gold state reads better next to crimson actions) and `GoldRule` (frame divider).
