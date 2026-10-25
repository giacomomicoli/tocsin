//! Tocsin design system for iced 0.14. The design documentation (guidelines, component
//! specs, HTML reference pages) lives in `.claude/skills/tocsin-design-system/`.
//!
//! Two themes ("Tocsin Void" dark, "Tocsin Sunwell" light). Every style fn reads the
//! token set of the active theme through `tokens(theme)`, so switching theme in
//! `App::theme()` restyles the whole app. Values mirror that folder's `tokens/*.css`
//! 1:1; change both together.

use iced::border::Radius;
use iced::gradient::{self, Gradient};
use iced::theme::Palette;
use iced::widget::{button, container, scrollable, text, text_input};
use iced::{color, font, Background, Border, Color, Degrees, Font, Shadow, Theme, Vector};

// ───────────────────────────── fonts ─────────────────────────────
// Register the bytes once in main.rs with `.font(theme::FONT_BYTES[i])`.
pub const FONT_BYTES: [&[u8]; 6] = [
    include_bytes!("../assets/fonts/Cinzel-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Cinzel-Bold.ttf"),
    include_bytes!("../assets/fonts/AlegreyaSans-Regular.ttf"),
    include_bytes!("../assets/fonts/AlegreyaSans-Medium.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"),
];

/// Body / UI text (set as the app default font).
pub const UI: Font = Font::with_name("Alegreya Sans");
pub const UI_MEDIUM: Font = Font {
    weight: font::Weight::Medium,
    ..UI
};
/// Titles, badges, primary + selected button labels.
pub const DISPLAY: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::with_name("Cinzel")
};
pub const DISPLAY_BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..DISPLAY
};
/// Versions, folder names, paths.
pub const MONO: Font = Font::with_name("JetBrains Mono");

// ───────────────────────────── logo ─────────────────────────────
pub const EMBLEM_PNG: &[u8] = include_bytes!("../assets/logo/tocsin-emblem-256.png");
pub const LOCKUP_PNG: &[u8] = include_bytes!("../assets/logo/tocsin-lockup-160h.png");

// ───────────────────────────── scale ─────────────────────────────
/// `text(..).size(theme::size::ROW)`
pub mod size {
    pub const XS: f32 = 11.0; // badges, sidebar section label
    pub const META: f32 = 12.0; // meta lines, small buttons
    pub const CONTROL: f32 = 13.0; // controls, installed version
    pub const CHILD: f32 = 14.0; // child row name
    pub const ROW: f32 = 15.0; // row name
    pub const SECTION: f32 = 16.0; // Settings section title
    pub const PAGE: f32 = 18.0; // page title
}
/// Spacing / padding steps. Paddings are `[vertical, horizontal]`.
pub mod space {
    pub const WINDOW: (f32, f32) = (1120.0, 700.0);
    pub const SIDEBAR_W: f32 = 240.0;
    pub const SIDEBAR_PAD: [u16; 4] = [18, 14, 14, 14]; // top right bottom left
    pub const CONTENT_PAD: u16 = 20;
    pub const PAGE_GAP: u32 = 14;
    pub const ROW_GAP: u32 = 8;
    pub const ROW_PAD: [u16; 2] = [12, 14];
    pub const ROW_CHILD_PAD: [u16; 2] = [10, 14];
    pub const CHILD_INDENT: f32 = 24.0;
    pub const BTN_SM: [u16; 2] = [5, 10];
    pub const BTN_MD: [u16; 2] = [8, 14];
    pub const CHIP: [u16; 2] = [6, 12];
    pub const BADGE: [u16; 2] = [3, 8];
    pub const CONFIRM_PAD: [u16; 2] = [12, 16];
    pub const ICON_HEAD: f32 = 32.0;
    pub const ICON_CHILD: f32 = 24.0;
    pub const VERSIONS_W: f32 = 120.0;
    pub const BADGE_COL_W: f32 = 150.0;
    pub const SEARCH_W: f32 = 240.0;
    pub const SOURCES_W: f32 = 400.0;
    pub const LOCKUP_H: f32 = 52.0;
    /// The mock's `scrollbar-width: thin`, as wide as its 6px list gutter.
    pub const SCROLLBAR_W: f32 = 6.0;
}
const R1: f32 = 2.0; // badges, rows
const R2: f32 = 3.0; // buttons, inputs
const R3: f32 = 4.0; // frames

// ───────────────────────────── tokens ─────────────────────────────
pub struct Tokens {
    pub bg_app: [Color; 2],
    pub surface_frame: [Color; 2],
    pub surface_row: Color,
    // Kept in step with tokens/colors.css; iced rows have no hover state.
    #[allow(dead_code)]
    pub surface_row_hover: Color,
    pub surface_child: Color,
    pub surface_sunken: Color,
    pub border_frame: Color,
    pub border_frame_inner: Color,
    pub border_row: Color,
    pub border_row_hover: Color,
    pub border_control: Color,
    pub text_1: Color,
    pub text_2: Color,
    pub text_3: Color,
    pub text_title: Color,
    pub text_gold: Color,
    pub accent: Color,
    pub focus: Color,
    pub update: Color,
    pub ok: Color,
    pub ahead: Color,
    pub manual: Color,
    pub error: Color,
    pub muted: Color,
    pub selected_bg: [Color; 2],
    pub selected_border: Color,
    pub selected_text: Color,
    pub ghost_hover: Color,
    pub input_bg: Color,
    pub rule: Color,
    pub shadow_frame: Shadow,
    // Kept in step with tokens/effects.css; iced inputs take no shadow.
    #[allow(dead_code)]
    pub glow: Shadow,
}

// Shared across themes: crimson primary + gold trim.
const PRIMARY_BG: [Color; 2] = [color!(0xa52b1d), color!(0x6a140c)];
const PRIMARY_BG_HOVER: [Color; 2] = [color!(0xc2392b), color!(0x7e1a10)];
const PRIMARY_BG_PRESSED: [Color; 2] = [color!(0x6a140c), color!(0x8a2014)];
const PRIMARY_BORDER: Color = color!(0xd4a84f);
const PRIMARY_TEXT: Color = color!(0xfff1c9);

pub static VOID: Tokens = Tokens {
    bg_app: [color!(0x140f22), color!(0x08060d)],
    surface_frame: [color!(0x1e1832), color!(0x120e1f)],
    surface_row: color!(0x161224),
    surface_row_hover: color!(0x1e1830),
    surface_child: color!(0x120f1d),
    surface_sunken: color!(0x0b0912),
    border_frame: color!(0x8a6626),
    border_frame_inner: color!(0xe8c477, 0.16),
    border_row: color!(0x2c2342),
    border_row_hover: color!(0x5e4418),
    border_control: color!(0x4a3c66),
    text_1: color!(0xece4d0),
    text_2: color!(0xa99fbd),
    text_3: color!(0x6f6688),
    text_title: color!(0xf6dc95),
    text_gold: color!(0xe8c477),
    accent: color!(0xd4a84f),
    focus: color!(0xe8c477),
    update: color!(0xff9a3c),
    ok: color!(0x5fd37a),
    ahead: color!(0xb98aff),
    manual: color!(0x5aa9ff),
    error: color!(0xff5f52),
    muted: color!(0x8a829c),
    selected_bg: [color!(0x3a2e12), color!(0x241b0a)],
    selected_border: color!(0xd4a84f),
    selected_text: color!(0xf6dc95),
    ghost_hover: color!(0xd4a84f, 0.08),
    input_bg: color!(0x0e0b16),
    rule: color!(0xd4a84f, 0.7),
    shadow_frame: Shadow {
        color: color!(0x000000, 0.55),
        offset: Vector::new(0.0, 6.0),
        blur_radius: 20.0,
    },
    glow: Shadow {
        color: color!(0xe8c477, 0.35),
        offset: Vector::new(0.0, 0.0),
        blur_radius: 10.0,
    },
};

pub static SUNWELL: Tokens = Tokens {
    bg_app: [color!(0xf7efdc), color!(0xebdfc2)],
    surface_frame: [color!(0xfdf9ef), color!(0xf3e9d2)],
    surface_row: color!(0xfbf6ea),
    surface_row_hover: color!(0xfffaf0),
    surface_child: color!(0xf4ead4),
    surface_sunken: color!(0xe9dcbc),
    border_frame: color!(0xb08636),
    border_frame_inner: color!(0x8a6626, 0.22),
    border_row: color!(0xdccaa0),
    border_row_hover: color!(0xb08636),
    border_control: color!(0xc8b386),
    text_1: color!(0x24190d),
    text_2: color!(0x5e4b30),
    text_3: color!(0x8a7556),
    text_title: color!(0x5a3f0e),
    text_gold: color!(0x7a5716),
    accent: color!(0x8a6626),
    focus: color!(0x8a6626),
    update: color!(0xb35400),
    ok: color!(0x1f7a36),
    ahead: color!(0x7a3dc0),
    manual: color!(0x1f62b0),
    error: color!(0xb3261e),
    muted: color!(0x7d7263),
    selected_bg: [color!(0xf6dc95), color!(0xe8c477)],
    selected_border: color!(0x8a6626),
    selected_text: color!(0x3d2a08),
    ghost_hover: color!(0x8a6626, 0.10),
    input_bg: color!(0xfffdf6),
    rule: color!(0x8a6626, 0.6),
    shadow_frame: Shadow {
        color: color!(0x5e4418, 0.18),
        offset: Vector::new(0.0, 4.0),
        blur_radius: 14.0,
    },
    glow: Shadow {
        color: color!(0xb08636, 0.35),
        offset: Vector::new(0.0, 0.0),
        blur_radius: 8.0,
    },
};

/// The token set behind an iced `Theme` built by [`void`] or [`sunwell`].
pub fn tokens(theme: &Theme) -> &'static Tokens {
    if theme.extended_palette().is_dark {
        &VOID
    } else {
        &SUNWELL
    }
}

fn palette(t: &Tokens) -> Palette {
    Palette {
        background: t.bg_app[1],
        text: t.text_1,
        primary: t.accent,
        success: t.ok,
        warning: t.update,
        danger: t.error,
    }
}
pub fn void() -> Theme {
    Theme::custom("Tocsin Void".to_string(), palette(&VOID))
}
pub fn sunwell() -> Theme {
    Theme::custom("Tocsin Sunwell".to_string(), palette(&SUNWELL))
}

// ───────────────────────────── helpers ─────────────────────────────
/// Top → bottom linear gradient. If it renders upside down on your iced build, swap the stops.
fn vgrad(c: [Color; 2]) -> Background {
    Background::Gradient(Gradient::Linear(
        gradient::Linear::new(Degrees(180.0))
            .add_stop(0.0, c[0])
            .add_stop(1.0, c[1]),
    ))
}
/// Left → right gradient fading out at both ends (the gold rule).
fn hfade(c: Color) -> Background {
    let clear = Color { a: 0.0, ..c };
    Background::Gradient(Gradient::Linear(
        gradient::Linear::new(Degrees(90.0))
            .add_stop(0.0, clear)
            .add_stop(0.5, c)
            .add_stop(1.0, clear),
    ))
}
fn alpha(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}
fn bd(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: 1.0,
        radius: Radius::from(radius),
    }
}

/// Status tone for badges, notes and the update-count tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Update,
    Ok,
    Ahead,
    Manual,
    Error,
    Muted,
}
impl Tone {
    pub fn color(self, t: &Tokens) -> Color {
        match self {
            Tone::Update => t.update,
            Tone::Ok => t.ok,
            Tone::Ahead => t.ahead,
            Tone::Manual => t.manual,
            Tone::Error => t.error,
            Tone::Muted => t.muted,
        }
    }
}

// ───────────────────────────── text styles ─────────────────────────────
// Use `.style(theme::muted)` instead of `.color(CONST)` so text follows the theme.
pub fn body(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).text_1),
    }
}
pub fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).text_2),
    }
}
pub fn faint(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).text_3),
    }
}
pub fn title(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).text_title),
    }
}
pub fn gold(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).text_gold),
    }
}
pub fn tone(t: Tone) -> impl Fn(&Theme) -> text::Style {
    move |th| text::Style {
        color: Some(t.color(tokens(th))),
    }
}

// ───────────────────────────── shell ─────────────────────────────
/// Window background.
pub fn app(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(vgrad(t.bg_app)),
        text_color: Some(t.text_1),
        ..Default::default()
    }
}
/// Left column: fixed 240px, never changes between pages.
pub fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(vgrad(tokens(theme).surface_frame)),
        ..Default::default()
    }
}
/// 1px gold column between sidebar and content (iced borders are all-sides only).
pub fn divider(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(tokens(theme).border_frame.into()),
        ..Default::default()
    }
}
/// 1px horizontal rule fading at both ends. Use with `.height(1)`.
pub fn rule_gold(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(hfade(tokens(theme).rule)),
        ..Default::default()
    }
}

// ───────────────────────────── frames & rows ─────────────────────────────
/// Gilded frame, outer layer: wrap `frame_inner` in this with `.padding(3)`.
pub fn frame_outer(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(vgrad(t.surface_frame)),
        border: bd(t.border_frame, R3),
        shadow: t.shadow_frame,
        ..Default::default()
    }
}
pub fn frame_inner(theme: &Theme) -> container::Style {
    container::Style {
        border: bd(tokens(theme).border_frame_inner, R1),
        ..Default::default()
    }
}

pub fn row(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(t.surface_row.into()),
        border: bd(t.border_row, R1),
        ..Default::default()
    }
}
pub fn row_child(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(t.surface_child.into()),
        border: bd(t.border_row, R1),
        ..Default::default()
    }
}
pub fn row_disabled(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(alpha(t.surface_row, 0.4).into()),
        border: bd(t.border_row, R1),
        text_color: Some(t.text_3),
        ..Default::default()
    }
}
pub fn row_missing(theme: &Theme) -> container::Style {
    container::Style {
        border: bd(alpha(tokens(theme).error, 0.5), R1),
        ..Default::default()
    }
}
/// "Replace N folders?" banner.
pub fn confirm(theme: &Theme) -> container::Style {
    let c = tokens(theme).update;
    container::Style {
        background: Some(alpha(c, 0.08).into()),
        border: bd(alpha(c, 0.6), R1),
        ..Default::default()
    }
}
/// 32/24px square before an addon name; put a monogram `text` or an `image` inside.
pub fn icon_slot(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(t.surface_sunken.into()),
        border: bd(t.border_frame, R1),
        ..Default::default()
    }
}
/// Status badge: `container(text(label.to_uppercase()).size(size::XS).font(DISPLAY)).padding(space::BADGE).style(badge(Tone::Ok))`.
pub fn badge(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let c = tone.color(tokens(theme));
        container::Style {
            background: Some(alpha(c, 0.12).into()),
            border: bd(alpha(c, 0.5), R1),
            text_color: Some(c),
            ..Default::default()
        }
    }
}
/// Flavor tag in Settings (gold, not a status).
pub fn tag_gold(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(alpha(t.accent, 0.10).into()),
        border: bd(t.border_frame, R1),
        text_color: Some(t.text_gold),
        ..Default::default()
    }
}
/// One loader diamond (use a "◆" text or a 6px square): accent when lit.
pub fn loader_dot(lit: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let t = tokens(theme);
        container::Style {
            background: Some((if lit { t.accent } else { t.border_control }).into()),
            ..Default::default()
        }
    }
}

// ───────────────────────────── buttons ─────────────────────────────
fn disabled_alpha(s: button::Status, mut st: button::Style) -> button::Style {
    if matches!(s, button::Status::Disabled) {
        st.text_color = alpha(st.text_color, 0.4);
        st.border.color = alpha(st.border.color, 0.4);
        if let Some(Background::Color(c)) = st.background {
            st.background = Some(alpha(c, 0.4).into());
        }
    }
    st
}
/// Crimson + gold. One per area. Label font: DISPLAY.
pub fn primary(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => PRIMARY_BG_HOVER,
        button::Status::Pressed => PRIMARY_BG_PRESSED,
        _ => PRIMARY_BG,
    };
    let mut st = button::Style {
        background: Some(vgrad(bg)),
        text_color: PRIMARY_TEXT,
        border: bd(PRIMARY_BORDER, R2),
        shadow: Shadow::default(),
        snap: false,
    };
    if matches!(status, button::Status::Disabled) {
        st.background = Some(vgrad([alpha(bg[0], 0.4), alpha(bg[1], 0.4)]));
        st.text_color = alpha(PRIMARY_TEXT, 0.4);
        st.border.color = alpha(PRIMARY_BORDER, 0.4);
    }
    st
}
/// Secondary / ghost. Label font: UI_MEDIUM.
pub fn ghost(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    let bg =
        matches!(status, button::Status::Hovered | button::Status::Pressed).then(|| t.ghost_hover.into());
    disabled_alpha(
        status,
        button::Style {
            background: bg,
            text_color: t.text_1,
            border: bd(t.border_control, R2),
            shadow: Shadow::default(),
            snap: false,
        },
    )
}
/// Remove, Replace. Label font: UI_MEDIUM.
pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    let c = tokens(theme).error;
    let bg =
        matches!(status, button::Status::Hovered | button::Status::Pressed).then(|| alpha(c, 0.16).into());
    disabled_alpha(
        status,
        button::Style {
            background: bg,
            text_color: c,
            border: bd(alpha(c, 0.55), R2),
            shadow: Shadow::default(),
            snap: false,
        },
    )
}
/// Active filter chip. Label font: DISPLAY.
pub fn selected(theme: &Theme, _status: button::Status) -> button::Style {
    let t = tokens(theme);
    button::Style {
        background: Some(vgrad(t.selected_bg)),
        text_color: t.selected_text,
        border: bd(t.selected_border, R2),
        shadow: Shadow::default(),
        snap: false,
    }
}
/// Sidebar page nav item (Library / Get addons / Settings). Full width, DISPLAY font.
pub fn nav_item(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if active {
            return selected(theme, status);
        }
        let t = tokens(theme);
        let bg =
            matches!(status, button::Status::Hovered | button::Status::Pressed).then(|| t.ghost_hover.into());
        button::Style {
            background: bg,
            text_color: t.text_2,
            border: bd(Color::TRANSPARENT, R2),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}
/// Sidebar installation item (Retail, Classic Era…).
pub fn install_item(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if active {
            return selected(theme, status);
        }
        let t = tokens(theme);
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: None,
            text_color: t.text_1,
            border: bd(if hover { t.border_row_hover } else { t.border_row }, R2),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}
/// "+ Add game folder" in the sidebar (iced has no dashed borders: solid, muted).
pub fn add_folder(theme: &Theme, status: button::Status) -> button::Style {
    let mut st = ghost(theme, status);
    st.text_color = tokens(theme).text_2;
    st
}

// ───────────────────────────── scrollbars ─────────────────────────────
/// Thin scroller in the control border color on a clear rail, accent while
/// hovered or dragged (the mock's `scrollbar-color: border-control transparent`).
pub fn scrollbar(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let t = tokens(theme);
    let engaged = matches!(
        status,
        scrollable::Status::Hovered {
            is_vertical_scrollbar_hovered: true,
            ..
        } | scrollable::Status::Dragged {
            is_vertical_scrollbar_dragged: true,
            ..
        }
    );
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: (if engaged { t.accent } else { t.border_control }).into(),
            border: Border {
                radius: Radius::from(R2),
                ..Border::default()
            },
        },
    };
    scrollable::Style {
        vertical_rail: rail,
        horizontal_rail: rail,
        ..scrollable::default(theme, status)
    }
}

// ───────────────────────────── inputs ─────────────────────────────
pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let t = tokens(theme);
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: t.input_bg.into(),
        border: bd(if focused { t.focus } else { t.border_control }, R2),
        icon: t.text_2,
        placeholder: t.text_3,
        value: t.text_1,
        selection: alpha(t.accent, 0.35),
    }
}
