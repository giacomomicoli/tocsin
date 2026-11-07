//! WoWInterface descriptions (`UIDescription` of
//! `GET /v3/game/WOW/filedetails/{id}.json`), written in the BBCode of its
//! forum: `[b]`, `[url=…]`, `[img]`, `[list]` / `[*]`, `[quote]`, `[code]`,
//! plus looks (`[color]`, `[size]`, `[center]`) that are dropped. Newlines
//! are meaningful: one breaks the line, two end the paragraph.

use super::doc::{Bases, Builder, Doc};
use super::html::decode_entities;

/// Convert `text` to a [`Doc`].
pub(crate) fn to_doc(text: &str) -> Doc {
    let mut b = Builder::new(Bases::default());
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let Some(open) = rest.find('[') else {
            lines(&mut b, rest);
            break;
        };
        lines(&mut b, &rest[..open]);
        rest = &rest[open..];
        rest = match Tag::read(rest) {
            Some((tag, after)) => apply(&mut b, &tag, after),
            None => {
                lines(&mut b, "[");
                &rest[1..]
            }
        };
    }
    b.finish()
}

/// Text with its newlines: one is a line break, a blank line a new
/// paragraph. WoWInterface stores some text HTML-escaped (`&quot;`).
fn lines(b: &mut Builder, text: &str) {
    let text = decode_entities(text);
    let mut newlines = 0;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            newlines += 1;
        }
        if line.trim().is_empty() {
            continue;
        }
        match newlines {
            0 => {}
            1 => b.line_break(),
            _ => b.flush(),
        }
        newlines = 0;
        b.text(line);
    }
    match newlines {
        0 => {}
        1 => b.line_break(),
        _ => b.flush(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Tag<'a> {
    name: String,
    closing: bool,
    value: Option<&'a str>,
}

impl<'a> Tag<'a> {
    /// `[b]`, `[/url]`, `[url=https://…]`, `[*]`: the tag `s` starts with,
    /// or `None` when the brackets hold something else ("[12.0] fixes").
    fn read(s: &'a str) -> Option<(Tag<'a>, &'a str)> {
        let close = s.find(']')?;
        let inner = &s[1..close];
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(i) => (true, i),
            None => (false, inner),
        };
        let (name, value) = match inner.split_once('=') {
            Some((n, v)) => (n, Some(v.trim().trim_matches(['"', '\'']))),
            None => (inner, None),
        };
        let name = name.trim().to_ascii_lowercase();
        let known = matches!(
            name.as_str(),
            "b" | "i"
                | "u"
                | "s"
                | "url"
                | "email"
                | "img"
                | "list"
                | "*"
                | "quote"
                | "code"
                | "php"
                | "html"
                | "color"
                | "size"
                | "font"
                | "center"
                | "left"
                | "right"
                | "indent"
                | "highlight"
                | "spoiler"
                | "h"
                | "noparse"
        );
        known.then_some((Tag { name, closing, value }, &s[close + 1..]))
    }
}

/// Apply `tag` and return what follows it. `[url]X[/url]`, `[img]X[/img]`
/// and the code tags read their content here.
fn apply<'a>(b: &mut Builder, tag: &Tag<'_>, after: &'a str) -> &'a str {
    let open = !tag.closing;
    match tag.name.as_str() {
        "b" | "h" => b.strong(open),
        "i" => b.emphasis(open),
        "url" if open => match tag.value {
            Some(href) => b.open_link(href),
            None => {
                let (href, rest) = content(after, "url");
                b.open_link(href.trim());
                b.text(href);
                b.close_link();
                return rest;
            }
        },
        "url" => b.close_link(),
        "img" if open => {
            let (src, rest) = content(after, "img");
            b.image(src.trim(), "");
            return rest;
        }
        "list" if open => {
            let ordered = tag.value.is_some_and(|v| !v.is_empty());
            let start = tag.value.and_then(|v| v.parse().ok()).unwrap_or(1);
            b.open_list(ordered, start);
        }
        "list" => b.close_list(),
        "*" if open => b.open_item(),
        "quote" if open => b.open_quote(),
        "quote" => b.close_quote(),
        "code" | "php" | "html" | "noparse" if open => {
            let (code, rest) = content(after, &tag.name);
            if tag.name == "noparse" {
                b.text(code);
            } else {
                b.open_pre();
                b.raw(&decode_entities(code));
                b.close_pre();
            }
            return rest;
        }
        _ => {}
    }
    after
}

/// The text up to `[/name]`, and what follows that tag.
fn content<'a>(s: &'a str, name: &str) -> (&'a str, &'a str) {
    let close = format!("[/{name}]");
    let lower = s.to_ascii_lowercase();
    match lower.find(&close) {
        Some(i) => (&s[..i], &s[i + close.len()..]),
        None => (s, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::super::doc::{plain, Block};
    use super::*;

    /// Deadly Boss Mods' `UIChangeLog` as `filedetails/8814.json` answered
    /// it in November 2026, shortened.
    const DBM: &str = "[size=5]DBM - Core[/size]\r\n[size=4][url=https://github.com/DeadlyBossMods/DeadlyBossMods/tree/11.2.15]11.2.15[/url] (2025-09-15)[/size]\r\n[list]\r\n[*]prep retag\r\n[*]Bugfixes;\r\n    - Make positioning cleaner\r\n[*]Update localization.ru.lua (#1759)\r\n[/list]\r\n\r\nMake sure you download the [b]correct[/b] version.";

    #[test]
    fn wowinterface_bbcode_keeps_lists_links_and_lines() {
        let doc = to_doc(DBM);
        assert_eq!(
            plain(&doc),
            [
                "DBM - Core\n11.2.15 (2025-09-15)",
                "* prep retag",
                "* Bugfixes;\n- Make positioning cleaner",
                "* Update localization.ru.lua (#1759)",
                "Make sure you download the correct version.",
            ]
        );
        let Block::Text { spans, .. } = &doc.blocks[0] else {
            panic!()
        };
        assert_eq!(
            spans.iter().find_map(|s| s.link.as_deref()),
            Some("https://github.com/DeadlyBossMods/DeadlyBossMods/tree/11.2.15")
        );
        let Block::Text { spans, .. } = &doc.blocks[4] else {
            panic!()
        };
        assert!(spans.iter().any(|s| s.strong && s.text == "correct"));
    }

    #[test]
    fn bare_urls_pictures_code_and_brackets_that_are_not_tags() {
        let doc = to_doc(
            "[12.0] Fixes for [url]https://www.wowinterface.com/forums[/url]\n\n[img]https://cdn-wow.mmoui.com/preview/pvw1.png[/img]\n[code]/dbm help\n/dbm   pull 10[/code][quote]Nice[/quote]",
        );
        assert_eq!(
            plain(&doc),
            [
                "[12.0] Fixes for https://www.wowinterface.com/forums",
                "img https://cdn-wow.mmoui.com/preview/pvw1.png",
                "code /dbm help\n/dbm   pull 10",
                "> Nice",
            ]
        );
    }

    #[test]
    fn broken_bbcode_never_panics() {
        for junk in [
            "[",
            "[url",
            "[url=]",
            "[img]",
            "[list=x][*]a",
            "[/quote][/list]x",
            "[code]open",
            "[é]ü",
        ] {
            let _ = to_doc(junk);
        }
        assert_eq!(plain(&to_doc("[list=x][*]a")), ["1. a"]);
    }
}
