# Using the Tocsin design system in the iced app

Target: `crates/tocsin` on **iced 0.14**. Goal: the running app matches `ui_kits/tocsin-desktop/Library.dc.html`.

## 1. Files to copy into the crate

```
crates/tocsin/
  src/theme.rs                      ← rust/theme.rs (replaces the current file)
  assets/logo/tocsin-emblem-256.png ← assets/logo/tocsin-emblem-256.png
  assets/logo/tocsin-lockup-160h.png← assets/logo/tocsin-lockup-160h.png
  assets/fonts/                     ← the 6 TTFs below
```

Fonts (all SIL OFL, free to ship). Download the **static** files from Google Fonts (`static/` folder in each zip — not the variable `[wght]` file):

| File | Family |
|---|---|
| `Cinzel-SemiBold.ttf`, `Cinzel-Bold.ttf` | fonts.google.com/specimen/Cinzel |
| `AlegreyaSans-Regular.ttf`, `AlegreyaSans-Medium.ttf` | fonts.google.com/specimen/Alegreya+Sans |
| `JetBrainsMono-Regular.ttf`, `JetBrainsMono-Medium.ttf` | fonts.google.com/specimen/JetBrains+Mono |

Add the three `OFL.txt` files next to them (licence requirement) and a line in the README.

## 2. Cargo.toml

```toml
# `image-without-codecs`: the sidebar logo widget, with no decoder of its own.
iced = { version = "0.14", features = ["tokio", "image-without-codecs"] }
# PNG decoding only, for the logo and the window icon.
image = { version = "0.25", default-features = false, features = ["png"] }
```

Do **not** use iced's `image` feature: it enables `image/default`, i.e. every codec (AVIF encoder, EXR, TIFF, GIF, …), which the repository's footprint rule (AGENTS.md, "Small footprint") forbids. The crate-level `image` dependency picks the codecs explicitly; `tocsin-core` adds `jpeg` for addon icons, and features unify. Check `cargo tree -e features -i image` after any change.

## 3. main.rs

```rust
use iced::{window, Size};

fn main() -> iced::Result {
    // (tracing setup omitted)
    let mut app = iced::application(boot, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .default_font(theme::UI)
        .window(window::Settings {
            size: Size::new(theme::space::WINDOW.0, theme::space::WINDOW.1),
            min_size: Some(Size::new(1000.0, 620.0)),
            // `from_file_data` needs iced's every-codec `image` feature:
            // decode the PNG with the `image` crate, then `from_rgba`.
            icon: window_icon(),
            ..Default::default()
        })
        .antialiasing(true);
    for bytes in theme::FONT_BYTES {
        app = app.font(bytes);
    }
    app.run()
}
```

## 4. Theme switch

```rust
// App state
dark: bool, // persist in tocsin.toml if you want it remembered

pub fn theme(&self) -> Theme {
    if self.dark { theme::void() } else { theme::sunwell() }
}
```

Every style fn calls `theme::tokens(theme)`, so nothing else changes when the theme flips.

## 5. Mechanical replacements in `app.rs`, `library.rs`, `discover.rs`, `settings.rs`

| Before | After |
|---|---|
| `.color(theme::TEXT_MUTED)` | `.style(theme::muted)` |
| `.color(theme::DANGER)` | `.style(theme::tone(Tone::Error))` |
| `.color(theme::WARNING)` | `.style(theme::tone(Tone::Update))` |
| `theme::badge(theme::ACCENT)` | `theme::badge(Tone::Update)` |
| `theme::badge(theme::SUCCESS)` | `theme::badge(Tone::Ok)` |
| `theme::badge(theme::WARNING)` (Ahead) | `theme::badge(Tone::Ahead)` |
| `theme::badge(theme::WARNING)` (Unknown/Manual) | `theme::badge(Tone::Manual)` |
| `theme::badge(theme::DANGER)` | `theme::badge(Tone::Error)` |
| `theme::badge(theme::TEXT_MUTED)` | `theme::badge(Tone::Muted)` |
| active chip `theme::primary` | `theme::selected` |
| `text_input(..)` | `text_input(..).style(theme::input)` |
| `theme::card` | gilded frame (§7) |

Badge labels: `text(label.to_uppercase()).size(size::XS).font(theme::DISPLAY)`, padding `space::BADGE`. Primary/selected button labels use `.font(theme::DISPLAY)`; ghost/danger use `theme::UI_MEDIUM`. Versions and paths use `theme::MONO`.

## 6. App shell (`App::view`)

Sidebar fixed, only the content column changes per `Screen`.

```rust
pub fn view(&self) -> Element<'_, Message> {
    let content: Element<_> = match self.screen {
        Screen::Library => self.library_page(),
        Screen::Discover => self.discover(),
        Screen::Settings => self.settings(),
    };
    let content = column![content, self.footer()]
        .spacing(theme::space::PAGE_GAP)
        .padding(theme::space::CONTENT_PAD)
        .width(Fill);

    container(row![
        self.sidebar(),
        container(space()).width(1).height(Fill).style(theme::divider),
        content,
    ])
    .width(Fill)
    .height(Fill)
    .style(theme::app)
    .into()
}

fn sidebar(&self) -> Element<'_, Message> {
    // `self.lockup` is made once in `boot`: a `Handle::from_bytes` here would
    // get a new id, and the PNG be decoded again, on every frame.
    let lockup = image(self.lockup.clone()).height(theme::space::LOCKUP_H);
    let rule = || container(space()).width(Fill).height(1).style(theme::rule_gold);

    let installs = self.manifest.settings.game_folders.iter().enumerate().fold(
        column![].spacing(6),
        |col, (i, f)| col.push(self.install_item(i, f)),
    );
    let nav = column![
        self.nav("Library", Screen::Library),
        self.nav("Get addons", Screen::Discover),
        self.nav("Settings", Screen::Settings),
    ]
    .spacing(4);

    container(
        column![
            lockup,
            rule(),
            text("INSTALLATIONS").size(theme::size::XS).font(theme::DISPLAY).style(theme::gold),
            installs,
            button(text("+ Add game folder").size(theme::size::META))
                .padding(theme::space::BTN_SM)
                .width(Fill)
                .style(theme::add_folder)
                .on_press(Message::AddGameFolder),
            space::vertical(),
            rule(),
            nav,
        ]
        .spacing(14),
    )
    .width(theme::space::SIDEBAR_W)
    .height(Fill)
    .padding(theme::space::SIDEBAR_PAD)
    .style(theme::sidebar)
    .into()
}

fn nav(&self, label: &'static str, screen: Screen) -> Element<'_, Message> {
    button(text(label).size(theme::size::CONTROL).font(theme::DISPLAY))
        .padding([7, 10])
        .width(Fill)
        .style(theme::nav_item(self.screen == screen))
        .on_press(Message::Show(screen))
        .into()
}
```

Install item: `button(row![icon_slot(monogram), column![name (DISPLAY 13), "build · N addons" (META, muted), path (MONO 10, faint)], update-count badge])` with `.style(theme::install_item(i == self.active_folder))` → `Message::SelectFolder(i)`. Shorten the path to its last two components yourself; iced does not ellipsize.

## 7. Recipes

**Page head**

```rust
column![
    row![
        column![
            text("Installed Addons").size(size::PAGE).font(theme::DISPLAY).style(theme::title),
            text(format!("{} · {}", self.client, dir.display())).size(size::META).font(theme::MONO).style(theme::muted),
        ].spacing(2).width(Fill),
        text(format!("{} addons", n)).size(size::META).style(theme::muted),
        rescan_btn, check_btn,
    ].spacing(14).align_y(Alignment::Center),
    container(space()).width(Fill).height(1).style(theme::rule_gold),
]
```

**Gilded frame** (Settings sections, any modal-level surface)

```rust
container(container(body).padding([16, 18]).style(theme::frame_inner))
    .padding(3)
    .style(theme::frame_outer)
```

**Icon slot** (every addon row and search result)

```rust
let s = if child { space::ICON_CHILD } else { space::ICON_HEAD };
container(text(monogram).size(if child { 11 } else { 14 }).font(theme::DISPLAY_BOLD).style(theme::gold))
    .width(s).height(s).center(s)
    .style(theme::icon_slot)
```

Swap the `text` for `image(handle)` when an icon exists. Dim missing/disabled/removed rows by using `theme::faint` for the monogram.

**Addon row**: unchanged structure from `library.rs`, plus the icon slot as the first item after `└`; padding `space::ROW_PAD` / `ROW_CHILD_PAD`; spacing 14; children indented `space::CHILD_INDENT`.

**Loader**: 3 × `container(text("◆").size(9))` (or 6px squares) styled `theme::loader_dot(i == tick % 3)`, as today.

## 8. Things iced can't do (and what the system does instead)

- Letter-spacing, text shadow, blur: not used anywhere.
- Dashed borders ("+ Add game folder" is dashed in the mock): use solid `theme::add_folder`.
- Text ellipsis: truncate strings before passing them to `text` (`ellipsize()` already exists in `discover.rs`).
- Hover border on rows: wrap the row in `mouse_area` + track the hovered folder, or skip it (minor).
- Gradient direction: if a gradient renders upside down on your build, swap the two stops in `vgrad`.
- Scrollbar: style with `scrollable(..).style(..)` using `tokens(theme).border_control` for the scroller if you want to match the mock.

## 9. As applied

`crates/tocsin/src/theme.rs` is now the source of truth and has moved on from `rust/theme.rs`: it adds `scrollbar` and `space::SCROLLBAR_W`, drops the unused `card` alias, and marks `surface_row_hover` / `glow` as kept for parity (iced rows have no hover state, inputs no shadow). When the crate's file changes, update `rust/theme.rs` here or treat it as historical. Addon icons (search and library) fill the icon slot through `tocsin_core::icons`, which decodes downloaded pictures; the GUI never decodes one itself.

## 10. Check

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p tocsin
```

Compare side by side with `ui_kits/tocsin-desktop/Library.dc.html` (tweak `theme` and `startScreen`).
