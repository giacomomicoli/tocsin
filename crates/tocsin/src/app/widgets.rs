//! Pieces every page shares: page head, gold rule, gilded frame, icon slot,
//! badge and the three button kinds. Sizes, paddings, colors and fonts all
//! come from `theme`; nothing here picks its own.

use iced::widget::{
    button, column, container, image, row, scrollable, space, text, tooltip, Button, Scrollable, Text,
};
use iced::{Alignment, Element, Fill, Font, Padding, Theme};

use super::Message;
use crate::theme::{self, size, Tone};

/// A `[top, right, bottom, left]` padding, which iced only takes as fields.
pub(super) fn pad4([top, right, bottom, left]: [u16; 4]) -> Padding {
    Padding {
        top: f32::from(top),
        right: f32::from(right),
        bottom: f32::from(bottom),
        left: f32::from(left),
    }
}

/// The 1px gold rule fading out at both ends.
pub(super) fn rule<'a>() -> Element<'a, Message> {
    container(space::horizontal())
        .width(Fill)
        .height(1)
        .style(theme::rule_gold)
        .into()
}

/// Page title over a mono subtitle (client, path), with `trailing` on the
/// right, then the gold rule.
pub(super) fn page_head<'a>(
    title: &'a str,
    subtitle: String,
    trailing: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let titles = column![
        text(title)
            .size(size::PAGE)
            .font(theme::DISPLAY)
            .style(theme::title),
        one_line(
            text(subtitle)
                .size(size::META)
                .font(theme::MONO)
                .style(theme::muted)
        ),
    ]
    .spacing(2);
    // Clipped: a long game path is cut at the edge instead of wrapping.
    let titles = container(titles).width(Fill).clip(true);
    let mut head = row![titles].spacing(14).align_y(Alignment::Center);
    if let Some(t) = trailing {
        head = head.push(t);
    }
    column![head, rule()].spacing(theme::space::PAGE_GAP).into()
}

/// Text that never wraps. iced cannot ellipsize, so put it in a clipped
/// container: what does not fit is cut at the edge, as in the mock.
pub(super) fn one_line<'a>(t: Text<'a>) -> Text<'a> {
    t.wrapping(text::Wrapping::None)
}

/// `s` in at most `max` characters, the last one `…` when it had to be cut.
pub(super) fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let Some(keep) = max.checked_sub(1) else {
        return String::new();
    };
    let kept: String = s.chars().take(keep).collect();
    format!("{}…", kept.trim_end())
}

/// A mono value in a column `width` wide: cut with `…` when it does not fit,
/// and then shown whole in a hover tip. iced cannot ellipsize, but a mono
/// font makes the cut exact by character count; the clip is the backstop.
/// `styled` adds the color, so a row's own (faded) text color still applies
/// when it adds none.
pub(super) fn mono_fitted<'a>(
    value: String,
    size: f32,
    width: f32,
    styled: impl FnOnce(Text<'a>) -> Text<'a>,
) -> Element<'a, Message> {
    let shown = ellipsize(&value, theme::mono_fit(width, size));
    let cut = shown != value;
    let line = container(styled(one_line(text(shown).size(size).font(theme::MONO))))
        .width(Fill)
        .align_x(Alignment::End)
        .clip(true);
    if !cut {
        return line.into();
    }
    let whole = container(text(value).size(size::META).font(theme::MONO))
        .padding(theme::space::TIP_PAD)
        .style(theme::hover_tip);
    tooltip(line, whole, tooltip::Position::Top)
        .gap(theme::space::HOVER_GAP)
        .into()
}

/// A vertical list with the thin themed scrollbar, 6px off the content.
pub(super) fn scroll<'a>(content: impl Into<Element<'a, Message>>) -> Scrollable<'a, Message> {
    let bar = scrollable::Scrollbar::new()
        .width(theme::space::SCROLLBAR_W)
        .scroller_width(theme::space::SCROLLBAR_W)
        .spacing(6);
    scrollable(content)
        .direction(scrollable::Direction::Vertical(bar))
        .height(Fill)
        .style(theme::scrollbar)
}

/// Gilded frame: the outer gold border, 3px, then the faint inner one.
pub(super) fn frame<'a>(body: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(
        container(body)
            .padding([16, 18])
            .width(Fill)
            .style(theme::frame_inner),
    )
    .padding(3)
    .style(theme::frame_outer)
    .into()
}

/// The square before an addon or installation name: its `picture` when
/// there is one, else its monogram. `dim` fades either for rows that are
/// missing, disabled or removed (the picture to half, as in the mock).
pub(super) fn icon_slot<'a>(
    mono: String,
    child: bool,
    dim: bool,
    picture: Option<image::Handle>,
) -> Element<'a, Message> {
    let (side, glyph) = if child {
        (theme::space::ICON_CHILD, size::XS)
    } else {
        (theme::space::ICON_HEAD, size::CHILD)
    };
    let inside: Element<'a, Message> = match picture {
        // Inside the slot's 1px border. The picture is a square already.
        Some(handle) => image(handle)
            .width(side - 2.0)
            .height(side - 2.0)
            .opacity(if dim { 0.5_f32 } else { 1.0 })
            .into(),
        None => {
            let style: fn(&Theme) -> text::Style = if dim { theme::faint } else { theme::gold };
            text(mono)
                .size(glyph)
                .font(theme::DISPLAY_BOLD)
                .style(style)
                .into()
        }
    };
    container(inside).center(side).style(theme::icon_slot).into()
}

/// Two letters standing for a name without an icon: its first two letters
/// or digits ("Details! Damage Meter" → "De").
pub(super) fn monogram(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).take(2).collect()
}

/// Status badge: uppercase display label tinted by `tone`.
pub(super) fn badge<'a>(label: &str, tone: Tone) -> Element<'a, Message> {
    container(text(label.to_uppercase()).size(size::XS).font(theme::DISPLAY))
        .padding(theme::space::BADGE)
        .style(theme::badge(tone))
        .into()
}

fn labelled<'a>(label: String, size: f32, font: Font) -> Button<'a, Message> {
    button(text(label).size(size).font(font))
}

/// Crimson + gold, display font: the one main action of an area.
pub(super) fn primary<'a>(label: impl Into<String>, size: f32, pad: [u16; 2]) -> Button<'a, Message> {
    labelled(label.into(), size, theme::DISPLAY)
        .padding(pad)
        .style(theme::primary)
}

/// Secondary action.
pub(super) fn ghost<'a>(label: impl Into<String>, size: f32, pad: [u16; 2]) -> Button<'a, Message> {
    labelled(label.into(), size, theme::UI_MEDIUM)
        .padding(pad)
        .style(theme::ghost)
}

/// Destructive action (Remove, Replace).
pub(super) fn danger<'a>(label: impl Into<String>, size: f32, pad: [u16; 2]) -> Button<'a, Message> {
    labelled(label.into(), size, theme::UI_MEDIUM)
        .padding(pad)
        .style(theme::danger)
}

/// The confirm banner: a question that blocks a destructive action, what
/// will happen, and the answers (`actions`, the destructive one last).
pub(super) fn confirm_banner<'a>(
    title: String,
    body: String,
    actions: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(
        row![
            column![
                text(title)
                    .size(size::CHILD)
                    .font(theme::DISPLAY)
                    .style(theme::tone(Tone::Update)),
                text(body).size(size::CONTROL).style(theme::body),
            ]
            .spacing(2)
            .width(Fill),
            actions.into(),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding(theme::space::CONFIRM_PAD)
    .style(theme::confirm)
    .into()
}

/// The "?" beside a badge whose reason is not obvious; `on_press` opens or
/// closes its [`tip`].
pub(super) fn help<'a>(open: bool, on_press: Message) -> Button<'a, Message> {
    labelled("?".into(), size::XS, theme::DISPLAY)
        .padding(theme::space::HELP)
        .style(theme::help(open))
        .on_press(on_press)
}

/// What a [`help`] button explains, in a box under the line it belongs to.
pub(super) fn tip<'a>(body: &'a str) -> Element<'a, Message> {
    container(text(body).size(size::META))
        .padding(theme::space::TIP_PAD)
        .width(Fill)
        .style(theme::tip)
        .into()
}

/// A filter chip or theme choice: selected (display font) or ghost.
pub(super) fn chip<'a>(label: impl Into<String>, active: bool) -> Button<'a, Message> {
    let font = if active { theme::DISPLAY } else { theme::UI_MEDIUM };
    let b = labelled(label.into(), size::CONTROL, font).padding(theme::space::CHIP);
    if active {
        b.style(theme::selected)
    } else {
        b.style(theme::ghost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monograms_take_the_first_two_letters_or_digits() {
        assert_eq!(monogram("Details! Damage Meter"), "De");
        assert_eq!(monogram("!BugGrabber"), "Bu");
        assert_eq!(monogram("4"), "4");
        assert_eq!(monogram("—"), "");
    }

    #[test]
    fn ellipsize_keeps_what_fits_and_cuts_the_rest() {
        assert_eq!(ellipsize("12.0.391", 16), "12.0.391");
        // A CurseForge file name as the version (issue #14).
        assert_eq!(ellipsize("AztarecHelper-2.9.0.zip", 16), "AztarecHelper-2…");
        assert_eq!(ellipsize("AztarecHelper-2.9.0.zip", 16).chars().count(), 16);
        assert_eq!(ellipsize("a · b c", 6), "a · b…");
        assert_eq!(ellipsize("ab", 0), "");
    }

    #[test]
    fn the_version_column_fits_its_mono_characters() {
        // 120px at 12px: 7.2px a glyph, 16 of them; at 13px, 15.
        assert_eq!(theme::mono_fit(theme::space::VERSIONS_W, size::META), 16);
        assert_eq!(theme::mono_fit(theme::space::VERSIONS_W, size::CONTROL), 15);
    }
}
