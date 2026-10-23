# Tocsin

Tocsin is a native desktop app in Rust that checks World of Warcraft addons
for updates at their original source. This file is the working guide for
coding agents and contributors: how the repository is laid out, which
commands to run, and which rules a change must respect. User-facing behaviour
is described in `README.md`; the internals and the security model are in
`docs/technical.md`.

## Project overview

The workspace has two crates:

- `crates/tocsin-core` — library, no GUI: `.toc` parsing, discovery heuristics,
  version comparison, update sources, families, secrets, addon icons
  (`icons.rs`: allowed hosts, capped download, limited decode, disk cache).
- `crates/tocsin` — the iced GUI. Thin: state, messages, views. Logic lives in core.
  `app.rs` holds state, `update` and the window frame; each page's view is a
  child module (`app/library.rs`, `app/discover.rs`, `app/settings.rs`), the
  shared pieces are in `app/widgets.rs`, the design system in `theme.rs`.

The design system itself (tokens, guidelines, components, the visual
reference of every page) is a project skill:
`.claude/skills/tocsin-design-system/` — start at its `SKILL.md`.

Other places worth knowing: `fixtures/README.md` (reference scenarios for
tests), `tocsin.example.toml` (config example), `docs/install-design.md`
(install and rollback design), `deny.toml` (dependency policy).

## Build and test commands

```sh
cargo test --workspace                                   # unit tests
cargo clippy --workspace --all-targets -- -D warnings    # must be clean
cargo fmt --all                                          # before committing
cargo run -p tocsin                                      # GUI (debug)
cargo run -p tocsin-core --example check -- fixtures/AddOns   # headless check
cargo run -p tocsin-core --example install -- <AddOns> <zip> [key]   # headless install/rollback
cargo run -p tocsin-core --example search -- <query> [retail|classic|vanilla]   # headless search
```

CI runs fmt, clippy with `-D warnings`, tests on Windows and Linux, and
`cargo deny` (advisories, licenses, bans). A PR that fails any of these is not
mergeable.

## Priorities and security considerations

When goals conflict, they rank in this order.

1. **Security.** The app reads a game folder and talks to public APIs. Nothing
   else. Keep it that way:
   - `#![forbid(unsafe_code)]` stays in every crate.
   - HTTPS only; every request goes through `http::Http` (timeouts, body cap,
     redirects checked hop by hop: same host for APIs, `download::ALLOWED_HOSTS`
     for downloads, `icons::ALLOWED_HOSTS` for addon icons). Never add another
     client outside it. A new host is a security change: say so in
     "Security posture" in `docs/technical.md`.
   - Every file read has a size cap and reads only well-known names. Never walk
     an addon tree recursively. Never follow symlinks. The icon cache counts:
     its files are read back through `icons::decode` under the same caps.
   - Images from the network are untrusted input: they are decoded only by
     `icons::decode` (format from the first bytes, PNG or JPEG only, side and
     memory limits), off the UI thread, and reach the GUI as RGBA. The GUI
     never decodes a downloaded file. Bundled assets (fonts, logos) are
     compiled in with `include_bytes!`; nothing is read from disk for them.
   - Sending data to a provider is a privacy change: what a request reveals
     about the user's addons (search terms, the CurseForge ids of the logo
     lookup) is listed in the "Security posture" section of `docs/technical.md`.
   - Only `https://` URLs reach the OS browser (`open::that_detached`).
   - Secrets come from the environment or the OS credential store. Never log
     them, never write them to a file, never put a key in the repo.
   - Writes go to `AddOns/<Folder>`, `AddOns/.tocsin-staging`,
     `AddOns/.tocsin-backup`, `AddOns/.tocsin-disabled`, the config dir
     (`tocsin.toml`) and the cache dir (`downloads/`, `icons/`), nowhere else
     (plus the one-time move of early builds' folders in `paths::migrate_legacy`); a cache file is written to a new
     temporary file and renamed, never through a link. `WTF` is read-only territory, always. Downloads are
     accepted only from `download::ALLOWED_HOSTS`; zips go through
     `archive::inspect` before `archive::extract`; folders move by rename,
     never copy, and every multi-step swap undoes itself on failure.
   - Validate anything that ends up in a URL (owner/repo, ids).
2. **Performance.** Startup must feel instant; checks run concurrently with a
   bound (`checker::MAX_CONCURRENCY`). Blocking I/O goes through
   `spawn_blocking`, never on the UI thread. Prefer borrowing over cloning; an
   `Arc<Vec<InstalledAddon>>` is shared, not copied.
3. **Small footprint.** Every dependency must earn its place. Before adding
   one: check the feature flags (disable defaults), check what it pulls in
   (`cargo tree -e features`), and write one comment line in `Cargo.toml`
   saying why it exists. rustls only — `deny.toml` bans OpenSSL. Image
   codecs are opt-in: iced takes `image-without-codecs` (its `image` feature
   turns on every codec, an AVIF encoder included) and the `image` crate
   only `png` and `jpeg`.

## Code style

- Idiomatic Rust, `rustfmt` defaults with `max_width = 110`.
- Functions are short and do one thing. If a function needs a paragraph to
  explain, split it or write the paragraph as a doc comment on it.
- Comments explain *why*, not *what*. Document heuristics with the real-world
  case that motivated them (see `discover.rs`, `family.rs`).
- Public items in `tocsin-core` carry doc comments (`missing_docs` is on).
- Errors: `thiserror` enums in core, `anyhow` only at the edges. Messages are
  written for the user who will see them in the GUI: short, no URLs, no
  internals.
- No panics on user data. `.toc` files, config files and API responses are
  untrusted input: parse leniently, fail per item, never crash the app.
- Serde structs for API responses: every field `#[serde(default)]` unless its
  absence makes the whole answer useless.

## Testing instructions

- Every heuristic gets a unit test with a real-shaped input (a `.toc` snippet,
  a release list, a changelog line). Use the Routine / Zygor cases in
  `fixtures/README.md` as the reference scenarios.
- Network is never touched in tests. Test the decoding and the decision, not
  the transport.
- `tempfile` for anything that needs a directory.

## Adding an update source

1. `sources/<name>.rs`: a `latest()` that takes `&Http`, the credential it
   needs, and returns `Release`. Lenient serde structs, documented endpoint.
2. A `SourceRef` variant (serde-tagged, so it round-trips through `tocsin.toml`).
3. `SourceRef::from_toc` / `candidates_from_toc`, `provider()`, `web_url()`,
   `is_usable()`, and the match in `fetch_latest`.
4. If it needs a key: a `Secret` variant with env var, account name, label and
   help URL. The Settings page picks it up automatically.
5. A README row and a test.

## Design system (mandatory for UI work)

Every design decision goes through Tocsin's design system,
`.claude/skills/tocsin-design-system/`. Read its `SKILL.md` and `readme.md`
**before** any of these, and follow the workflow there:

- changing a view that exists (Library, Get addons, Settings, the shell);
- adding a new view, page, dialog or panel;
- adding a new UI element (a control, a row type, a status, a banner);
- changing the design or any UI element (a color, a size, a font, a look).

No view invents a color, size, padding, radius, font or component. If the
system lacks what a change needs, extend the system first (tokens in both
themes, a style function in `theme.rs`, a helper in `app/widgets.rs`, a
component spec in the skill) and then use it. A request that breaks the
system is answered with what it offers instead; the system is extended only
with the user's agreement. Agents that load skills from `.claude/skills`
get it automatically; others read the files directly.

Design stays out of the logic: colors, fonts, layout and iced belong to
`crates/tocsin` only; `crates/tocsin-core` never knows about them, and a view
that needs something computed gets a core function (with a test), never
logic of its own.

## GUI conventions

- `App::update` is the only place state changes; views are pure functions of
  `&self`. Long work returns a `Task::perform`.
- Styles live in `theme.rs`, which implements the design system skill (the
  crate's file wins where the two differ; keep the skill's `tokens/*.css`
  1:1 with it). Colors come from `theme::tokens(theme)` through the style functions there,
  so both themes (Void, Sunwell) follow; sizes and paddings from
  `theme::size` / `theme::space`. No color, size or font literals in views.
  Build rows from `app/widgets.rs` (page head, frame, badge, buttons, icon
  slot) rather than restyling by hand.
- Every status the user can see has a label in `UpdateStatus::label()` and a
  tone in `addon_row`. Add both when adding a status.
- Image handles are made once (in state, or when an icon arrives), never in
  `view`: `image::Handle::from_bytes` gets a new id per call and the picture
  would be decoded again every frame.

## Working on Windows

- Source files are UTF-8 without BOM, and UI strings use `·`, `…`, `—`, `└`.
  Do not rewrite files through Windows PowerShell 5.1 `Get-Content` /
  `Set-Content`: it reads them as Windows-1252 and saves mojibake that
  compiles and passes every test. Edit them with an editor or a tool that
  keeps UTF-8.
- Smart App Control can block unsigned build outputs (a proc-macro DLL, a
  test binary) at random. That is the OS policy, not the code: rebuild the
  blocked package (`cargo clean -p <crate>`) and run again, or rely on CI.

## Don'ts

- No telemetry, no auto-update of Tocsin itself, no background services.
- No registry or drive scanning: the user picks the game folder.
- No scraping of web pages; APIs only, and only the documented ones. Icons
  come only from URLs those APIs return, never from a page.
- No embedded API keys, however obfuscated.
- Don't widen what the app reads or writes on disk without saying so in the
  "Security posture" section of `docs/technical.md`.

## Commit and pull request guidelines

- One topic per commit, imperative subject, body says *why* when non-obvious.
- A PR touching security-relevant code (`http.rs`, `secrets.rs`, `download.rs`,
  `archive.rs`, `install.rs`, `icons.rs`, anything that reads files, decodes
  downloaded data or opens URLs) says so in its description.
- Update `README.md` when behaviour visible to the user changes.
- A PR touching the UI says which parts of the design system it uses, and
  lists any token, style or component it adds to it (and updates the skill
  in the same PR).
