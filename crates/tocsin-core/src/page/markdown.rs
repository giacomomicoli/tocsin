//! Markdown descriptions: Tukui's (`desc` of `GET /v1/addon/{slug}`), parsed
//! by `pulldown-cmark` into the same [`Builder`] events as HTML. The HTML a
//! Markdown file may hold goes through [`super::html::feed`].

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::doc::{Bases, Builder, Doc};
use super::html;

/// Convert `md` to a [`Doc`], resolving relative links against `bases`.
pub(crate) fn to_doc(md: &str, bases: Bases) -> Doc {
    let mut b = Builder::new(bases);
    let mut image: Option<(String, String)> = None;
    let mut cells = 0u16;
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    for event in Parser::new_ext(md, options) {
        // Inside `![alt](src)` the text is the alternative text.
        if let Some((_, alt)) = &mut image {
            match event {
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    if let Some((src, alt)) = image.take() {
                        b.image(&src, &alt);
                    }
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph | Tag::HtmlBlock => b.flush(),
                Tag::Heading { level, .. } => b.open_heading(level_of(level)),
                Tag::BlockQuote(_) => b.open_quote(),
                Tag::CodeBlock(_) => b.open_pre(),
                Tag::List(start) => {
                    let first = start.map(|n| u32::try_from(n).unwrap_or(u32::MAX));
                    b.open_list(first.is_some(), first.unwrap_or(1));
                }
                Tag::Item => b.open_item(),
                Tag::Emphasis => b.emphasis(true),
                Tag::Strong => b.strong(true),
                Tag::Link { dest_url, .. } => b.open_link(&dest_url),
                Tag::Image { dest_url, .. } => image = Some((dest_url.into_string(), String::new())),
                Tag::TableRow | Tag::TableHead => {
                    b.flush();
                    cells = 0;
                }
                Tag::TableCell => {
                    if cells > 0 {
                        b.text(" · ");
                    }
                    cells = cells.saturating_add(1);
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::HtmlBlock | TagEnd::TableRow | TagEnd::TableHead => b.flush(),
                TagEnd::Heading(_) => b.close_heading(),
                TagEnd::BlockQuote(_) => b.close_quote(),
                TagEnd::CodeBlock => b.close_pre(),
                TagEnd::List(_) => b.close_list(),
                TagEnd::Item => b.close_item(),
                TagEnd::Emphasis => b.emphasis(false),
                TagEnd::Strong => b.strong(false),
                TagEnd::Link => b.close_link(),
                _ => {}
            },
            Event::Text(t) => b.text(&t),
            Event::Code(c) => b.code_span(&c),
            Event::Html(h) | Event::InlineHtml(h) => html::feed(&mut b, &h),
            Event::SoftBreak => b.text(" "),
            Event::HardBreak => b.line_break(),
            Event::Rule => b.rule(),
            _ => {}
        }
    }
    b.finish()
}

fn level_of(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::super::doc::{plain, Block};
    use super::*;

    /// The start of ElvUI's `desc` as `api.tukui.org/v1/addon/elvui`
    /// answered it in November 2026.
    const ELVUI: &str = "## About\n\n**ElvUI** is a full user interface replacement for World of Warcraft, which means it completely replaces the Blizzard default UI.\n\n## Modules\n\n- **General** [e.g. Fonts, Textures]\n- **ActionBars** [Player, Pet]\n\n## Skins\n\n- *(Optional)* Remove Blizzard art\n\n![Unit frames](https://storage.tukui.org/ElvUI-2.jpg)\n\nSee the [wiki](https://github.com/tukui-org/ElvUI/wiki) or run `/ec`.\n\n<p align=\"center\"><img src=\"https://storage.tukui.org/ElvUI-Logo.png\"></p>\n\n1. one\n2. two\n\n---\n\n```\n/ec install\n```\n";

    #[test]
    fn tukui_markdown_reads_like_the_site() {
        let doc = to_doc(ELVUI, Bases::same("https://tukui.org/"));
        assert_eq!(
            plain(&doc),
            [
                "h2 About",
                "ElvUI is a full user interface replacement for World of Warcraft, which means it completely replaces the Blizzard default UI.",
                "h2 Modules",
                "* General [e.g. Fonts, Textures]",
                "* ActionBars [Player, Pet]",
                "h2 Skins",
                "* (Optional) Remove Blizzard art",
                "img https://storage.tukui.org/ElvUI-2.jpg",
                "See the wiki or run /ec.",
                "img https://storage.tukui.org/ElvUI-Logo.png",
                "1. one",
                "2. two",
                "---",
                "code /ec install",
            ]
        );
        let Some(Block::Image(i)) = doc.blocks.get(7) else {
            panic!()
        };
        assert_eq!(i.alt, "Unit frames");
        // Markdown declares no size: the page reserves its default height.
        assert_eq!((i.width, i.height), (None, None));
    }
}
