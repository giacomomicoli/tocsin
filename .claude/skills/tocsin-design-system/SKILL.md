---
name: tocsin-design-system
description: Tocsin's design system (Void / Sunwell themes, Cinzel + Alegreya Sans + JetBrains Mono, gilded frames, status tones, iced 0.14 styles). Use it when changing the user interface in crates/tocsin - editing a view, adding a page or element, or changing the look of anything - and for HTML mocks of the app.
user-invocable: true
---

# Tocsin design system

Every visual decision in Tocsin goes through this system. That covers four
kinds of request, and the same rule applies to all of them:

1. **Changing a view that exists** (Library, Get addons, Settings, the shell).
2. **Adding a new view** (a page, a dialog, a panel).
3. **Adding a new element** (a control, a row type, a status, a banner).
4. **Changing the design itself** (a color, a size, a font, a component's look).

No change to the GUI invents a color, a size, a padding, a radius, a font or
a component of its own. If the system has it, use it. If it does not, extend
the system first (step 4 below), then use it.

## Read first

- `readme.md` — product context, **the iced rule** (what any style may be),
  content voice, visual foundations, the app shell, iconography, logo, addon
  icons.
- The code it is implemented in (the reference when the two disagree):
  - `crates/tocsin/src/theme.rs` — tokens of both themes (`tokens(theme)`),
    `size::*`, `space::*`, fonts, every style function.
  - `crates/tocsin/src/app/widgets.rs` — page head, gold rule, gilded frame,
    icon slot, badge, primary / ghost / danger buttons, chips, one-line text,
    themed scroll.
- `ui_kits/tocsin-desktop/Library.dc.html` — the visual reference for all
  three pages, both themes (open it in a browser; tweak `startScreen`,
  `theme`).
- `components/*/*.prompt.md` — when to use each component.
- `tokens/*.css` — the same tokens as CSS, for mocks; kept 1:1 with
  `theme.rs`.

## Workflow

### 1. Changing a view that exists

- Compose from `widgets.rs` and style with the functions in `theme.rs`:
  `.style(theme::muted)`, `theme::badge(Tone::Ok)`, `widgets::primary(..)`.
- Sizes from `theme::size`, paddings and widths from `theme::space`. No
  `Color`, `color!`, pixel or font literals in a view. A literal that the
  mock specifies and the scale lacks (the 10px sidebar path) is the
  exception to name, not to copy.
- Keep the shell: the 240px sidebar never changes between pages; only the
  content column does.
- Copy follows the content rules in `readme.md`: sentence case, ` · `
  between facts, `…` on progress labels, status labels from
  `UpdateStatus::label()`, errors short and without internals, no emoji, no
  lore-speak.

### 2. Adding a new view

- It lives in its own child module of `app` (`app/<page>.rs`), as the
  existing pages do, with its entry point `pub(super)`.
- Its content column is: `widgets::page_head` (title, mono subtitle, page
  actions on the right) → its body → the footer notice, which `App::view`
  adds. Sections that need a surface use `widgets::frame`, as Settings does.
- A new page gets a nav item in the sidebar (`App::nav`) and a `Screen`
  variant; a modal-level question uses the confirm banner, not a new kind
  of overlay.
- Sketch it in the UI kit first (copy `Library.dc.html`'s structure) when
  the layout is not obvious, and show it to the user before building.

### 3. Adding a new element

- Look for it first: `readme.md` "Index", `components/`, `widgets.rs`. Most
  needs are a variant of a button, badge, row, frame or notice.
- If it is new, it must pass **the iced rule** (one background, one border,
  one shadow, a text color; no blur, letter-spacing, transforms or
  transitions). Then extend the system, in this order:
  1. tokens it needs, in `tokens/*.css` **and** `theme.rs` `VOID` + `SUNWELL`
     (both themes, always);
  2. a style function in `theme.rs` that reads `tokens(theme)`;
  3. a helper in `widgets.rs` if it will be used more than once;
  4. a component spec here: `components/<group>/<Name>.prompt.md` (and its
     card next to the others);
  5. a line in `readme.md` "Index".
- A new status needs a label in `UpdateStatus::label()` (core) and a `Tone`
  in `addon_row` (GUI).

### 4. Changing the design itself

- A change is made to the system, never to one view: edit the token or the
  style function, so every place that uses it follows.
- Change it in both themes and in both places (`tokens/*.css` and
  `theme.rs`), keep the names, and update the guideline card that shows it
  (`guidelines/*.card.html`) and `readme.md`.
- Check contrast in Void and Sunwell (switch in Settings → Theme).
- A request that breaks the system (a pill shape, a gradient text, a new
  font, an emoji, Blizzard art) is answered with what the system offers
  instead; extend the system only when the user agrees.

## Design stays out of the logic

The GUI/core split is part of this system:

- Design lives in `crates/tocsin` only: `theme.rs`, `app/widgets.rs` and the
  views. Views are pure functions of `&self`: no I/O, no decisions about
  addons, sources or files.
- `crates/tocsin-core` knows nothing about colors, fonts, layout or iced.
  If a view needs something computed (a count, a choice, a status), add a
  function to the core, with a test, and call it.
- Pictures shown in the UI come decoded from the core
  (`tocsin_core::icons`); the GUI never fetches or decodes a file. Image
  handles are made once, never in `view`.

## Before you finish

- Both themes look right; nothing unthemed (search the diff for `color!`,
  `Color::`, `.size(` with a number, `.padding(` with a number).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- On Windows, files stay UTF-8 without BOM: UI strings use `·`, `…`, `—`,
  `└`. Edit them with an editor that keeps UTF-8; rewriting through Windows
  PowerShell 5.1 `Get-Content` / `Set-Content` saves mojibake.

## Mocks and prototypes

For throwaway visuals (a mock for the user, a variant to compare), build a
static HTML page from `styles.css`, `tokens/` and the components here, and
copy the assets you use next to it. Keep it inside the iced rule, so what
the user approves can be built.
