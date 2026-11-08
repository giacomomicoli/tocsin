//! HTML descriptions: CurseForge's (`GET /v1/mods/{id}/description`, written
//! in its site editor) and GitHub's READMEs, rendered by GitHub
//! (`application/vnd.github.html`).
//!
//! Not a browser: a tokenizer that walks the text once, left to right, and
//! knows the few dozen tags descriptions use. Unknown tags are dropped and
//! their text kept; `<script>`, `<style>` and `<svg>` are dropped whole.
//! Nothing is executed, fetched or styled; attributes other than `href`,
//! `src`, `alt`, `start`, `data-canonical-src`, `width` and `height` are
//! not even kept.

use std::borrow::Cow;

use super::doc::{Bases, Builder, Doc};
use super::media::MAX_SIDE;

/// Convert `html` to a [`Doc`], resolving relative links against `bases`.
pub(crate) fn to_doc(html: &str, bases: Bases) -> Doc {
    let mut b = Builder::new(bases);
    feed(&mut b, html);
    b.finish()
}

/// Run `html` through `b`. Also used for the HTML Markdown lets in.
pub(crate) fn feed(b: &mut Builder, html: &str) {
    let mut state = State::default();
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            b.text(&decode_entities(rest));
            break;
        };
        if lt > 0 {
            b.text(&decode_entities(&rest[..lt]));
        }
        rest = &rest[lt..];
        rest = match Tag::read(rest) {
            Some((tag, after)) => {
                if let Some(raw_end) = state.apply(b, &tag) {
                    skip_raw(after, raw_end)
                } else {
                    after
                }
            }
            // A lone `<` ("a < b") or a comment: text, or nothing.
            None => match rest.strip_prefix("<!--") {
                Some(comment) => comment.find("-->").map_or("", |i| &comment[i + 3..]),
                None => {
                    b.text("<");
                    &rest[1..]
                }
            },
        };
    }
}

/// Past the closing `</name` of a raw-text element (`<script>`, `<style>`),
/// whose content is never looked at.
fn skip_raw<'a>(s: &'a str, name: &str) -> &'a str {
    let close = format!("</{name}");
    match find_ascii_ci(s, &close) {
        Some(i) => {
            let after = &s[i..];
            after.find('>').map_or("", |j| &after[j + 1..])
        }
        None => "",
    }
}

fn find_ascii_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.len() > h.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// One tag, as far as a description needs it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Tag {
    name: String,
    closing: bool,
    href: Option<String>,
    src: Option<String>,
    alt: Option<String>,
    start: Option<String>,
    canonical: Option<String>,
    width: Option<String>,
    height: Option<String>,
}

impl Tag {
    /// Read the tag `s` starts with (`s` starts with `<`): the tag and what
    /// follows it, or `None` when `<` starts no tag (a comment, a doctype,
    /// a lone `<`). A tag that never ends swallows the rest.
    fn read(s: &str) -> Option<(Tag, &str)> {
        let body = s.strip_prefix('<')?;
        let (closing, body) = match body.strip_prefix('/') {
            Some(b) => (true, b),
            None => (false, body),
        };
        let name_len = body
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(body.len());
        if name_len == 0 || !body.starts_with(|c: char| c.is_ascii_alphabetic()) {
            if let Some(decl) = body.strip_prefix('!').or_else(|| body.strip_prefix('?')) {
                if !decl.starts_with("--") {
                    return Some((Tag::default(), decl.find('>').map_or("", |i| &decl[i + 1..])));
                }
            }
            return None;
        }
        let mut tag = Tag {
            name: body[..name_len].to_ascii_lowercase(),
            closing,
            ..Tag::default()
        };
        let mut rest = &body[name_len..];
        loop {
            rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
            if rest.is_empty() {
                return Some((tag, rest));
            }
            if let Some(after) = rest.strip_prefix('>') {
                return Some((tag, after));
            }
            let name_end = rest
                .find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/'))
                .unwrap_or(rest.len());
            let name = rest[..name_end].to_ascii_lowercase();
            rest = rest[name_end..].trim_start();
            let mut value = None;
            if let Some(v) = rest.strip_prefix('=') {
                let v = v.trim_start();
                let (raw, after) = match v.chars().next() {
                    Some(q @ ('"' | '\'')) => {
                        let inner = &v[1..];
                        match inner.find(q) {
                            Some(end) => (&inner[..end], &inner[end + 1..]),
                            None => (inner, ""),
                        }
                    }
                    _ => {
                        let end = v.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(v.len());
                        (&v[..end], &v[end..])
                    }
                };
                value = Some(decode_entities(raw).into_owned());
                rest = after;
            }
            // A value with no name (`<p ="x">`): the value was read past.
            if name.is_empty() {
                continue;
            }
            let slot = match name.as_str() {
                "href" => &mut tag.href,
                "src" => &mut tag.src,
                "alt" => &mut tag.alt,
                "start" => &mut tag.start,
                "data-canonical-src" => &mut tag.canonical,
                "width" => &mut tag.width,
                "height" => &mut tag.height,
                _ => continue,
            };
            if slot.is_none() {
                *slot = value;
            }
        }
    }
}

/// What the tokenizer tracks across tags: table cells, to put ` · `
/// between them.
#[derive(Debug, Default)]
struct State {
    cells: u16,
}

impl State {
    /// Apply `tag` to `b`. Returns the element name when its content is raw
    /// text to skip unread (`<script>`, `<style>`).
    fn apply(&mut self, b: &mut Builder, tag: &Tag) -> Option<&'static str> {
        let open = !tag.closing;
        match tag.name.as_str() {
            "script" if open => return Some("script"),
            "style" if open => return Some("style"),
            "textarea" if open => return Some("textarea"),
            "title" if open => return Some("title"),
            "svg" | "noscript" | "template" | "head" | "select" | "button" => b.skip(open),
            "p" | "div" | "section" | "article" | "center" | "figure" | "figcaption" | "header"
            | "footer" | "main" | "nav" | "aside" | "table" | "thead" | "tbody" | "tfoot" | "details"
            | "dl" | "dd" | "dt" | "caption" => b.flush(),
            "summary" => {
                b.flush();
                b.strong(open);
            }
            "tr" => {
                b.flush();
                self.cells = 0;
            }
            "td" | "th" if open => {
                if self.cells > 0 {
                    b.text(" · ");
                }
                self.cells = self.cells.saturating_add(1);
                if tag.name == "th" {
                    b.strong(true);
                }
            }
            "th" => b.strong(false),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = tag.name.as_bytes()[1] - b'0';
                if open {
                    b.open_heading(level);
                } else {
                    b.close_heading();
                }
            }
            "ul" | "ol" if open => {
                let start = tag
                    .start
                    .as_deref()
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(1);
                b.open_list(tag.name == "ol", start);
            }
            "ul" | "ol" => b.close_list(),
            "li" if open => b.open_item(),
            "li" => b.close_item(),
            "blockquote" if open => b.open_quote(),
            "blockquote" => b.close_quote(),
            "pre" if open => b.open_pre(),
            "pre" => b.close_pre(),
            "code" | "kbd" | "samp" | "tt" => b.code(open),
            "strong" | "b" => b.strong(open),
            "em" | "i" | "cite" => b.emphasis(open),
            "a" if open => b.open_link(tag.href.as_deref().unwrap_or_default()),
            "a" => b.close_link(),
            "img" if open => {
                // GitHub proxies a README's outside pictures through its
                // camo host; the original address says whether it was an
                // SVG badge (shields.io), which is left out.
                let svg = tag.canonical.as_deref().is_some_and(|c| {
                    c.split(['?', '#'])
                        .next()
                        .unwrap_or(c)
                        .to_ascii_lowercase()
                        .ends_with(".svg")
                });
                if !svg {
                    b.sized_image(
                        tag.src.as_deref().unwrap_or_default(),
                        tag.alt.as_deref().unwrap_or_default(),
                        tag.width.as_deref().and_then(declared_side),
                        tag.height.as_deref().and_then(declared_side),
                    );
                }
            }
            "br" => b.line_break(),
            "hr" => b.rule(),
            // A YouTube player on CurseForge, a clip on GitHub: a link to
            // it, since nothing here plays video.
            "iframe" | "video" if open => {
                if let Some(src) = tag.src.as_deref() {
                    b.flush();
                    b.open_link(src);
                    b.text("Watch the video");
                    b.close_link();
                    b.flush();
                }
                if tag.name == "iframe" {
                    return Some("iframe");
                }
            }
            _ => {}
        }
        None
    }
}

/// A picture's `width` or `height` attribute, when it is a plain size in
/// pixels: CurseForge's editor writes `width="600"`, a README `width="300"`
/// or now and then `width="300px"`. A percentage (`width="100%"`, common in
/// READMEs), zero, a size past [`MAX_SIDE`] (no picture that large is ever
/// decoded) or anything else says nothing usable: `None`.
fn declared_side(raw: &str) -> Option<u32> {
    let raw = raw.trim();
    let digits = raw.strip_suffix("px").unwrap_or(raw).trim_end();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|&n| n > 0 && n <= MAX_SIDE)
}

/// `&amp;`, `&#39;`, `&#x2014;` and the named entities descriptions use,
/// decoded; anything else is left as written.
pub(crate) fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest
            .char_indices()
            .take(12)
            .find(|&(_, c)| c == ';')
            .map(|(j, _)| j);
        let decoded = end.and_then(|j| entity(&rest[1..j]).map(|c| (c, j)));
        match decoded {
            Some((c, j)) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return match code {
            0 => None,
            c => Some(char::from_u32(c).unwrap_or('\u{FFFD}')),
        };
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "times" => '×',
        "deg" => '°',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::doc::{plain, Block, Kind};
    use super::*;

    /// Shaped like Details!' CurseForge description (its editor wraps
    /// everything in `<p>`, styles headings by hand and embeds YouTube).
    const CURSEFORGE: &str = r#"<p style="text-align: center;"><img src="https://media.forgecdn.net/attachments/description/61284/description_7d1c.png" alt="Details!" width="600"></p>
<h2><span style="color: #e67e22;">Features</span></h2>
<ul>
<li><strong>Damage</strong>, healing &amp; threat meters.</li>
<li>Plugins:<ul><li>Encounter Details</li><li>Raid Check</li></ul></li>
</ul>
<p>Questions? Visit our <a href="https://discord.gg/details" rel="nofollow">Discord</a> or the <a href="http://www.curseforge.com/wow/addons/details/issues">issue tracker</a>.</p>
<p><iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ" width="560" height="315"></iframe></p>
<script>alert("no")</script><p>Thanks&nbsp;&mdash; Terciob</p>"#;

    #[test]
    fn curseforge_descriptions_keep_their_structure() {
        let doc = to_doc(CURSEFORGE, Bases::same("https://www.curseforge.com/"));
        assert_eq!(
            plain(&doc),
            [
                "img https://media.forgecdn.net/attachments/description/61284/description_7d1c.png",
                "h2 Features",
                "* Damage, healing & threat meters.",
                "* Plugins:",
                "  * Encounter Details",
                "  * Raid Check",
                "Questions? Visit our Discord or the issue tracker.",
                "Watch the video",
                "Thanks — Terciob",
            ]
        );
        let Block::Text { spans, .. } = &doc.blocks[6] else {
            panic!()
        };
        let links: Vec<_> = spans.iter().filter_map(|s| s.link.as_deref()).collect();
        assert_eq!(
            links,
            [
                "https://discord.gg/details",
                "https://www.curseforge.com/wow/addons/details/issues"
            ]
        );
    }

    /// Shaped like WeakAuras2's README as GitHub renders it: heading
    /// anchors with an SVG icon, badges, a camo-proxied logo, code.
    const README: &str = r##"<div class="markdown-heading" dir="auto"><h1 class="heading-element" dir="auto">WeakAuras</h1><a id="user-content-weakauras" class="anchor" aria-label="Permalink: WeakAuras" href="#weakauras"><svg class="octicon octicon-link" viewBox="0 0 16 16"><path d="m7.775 3.275"></path></svg></a></div>
<p dir="auto"><a href="https://github.com/WeakAuras/WeakAuras2/actions"><img src="https://github.com/WeakAuras/WeakAuras2/workflows/CI/badge.svg" alt="Build Status" style="max-width: 100%;"></a> <a href="https://discord.gg/weakauras" rel="nofollow"><img src="https://camo.githubusercontent.com/cb7f/68747470" alt="WeakAuras on Discord" data-canonical-src="https://img.shields.io/badge/discord-weakauras-738bd7.svg?style=flat" style="max-width: 100%;"></a></p>
<p dir="auto"><a target="_blank" rel="noopener noreferrer nofollow" href="https://camo.githubusercontent.com/e488/6874"><img src="https://camo.githubusercontent.com/e488/6874" alt="Logo" data-canonical-src="https://i.imgur.com/wwbxeCG.jpeg" style="max-width: 100%;"></a></p>
<p dir="auto">Install with <code>/wa</code> in game, or see <a href="/WeakAuras/WeakAuras2/wiki">the wiki</a>.</p>
<div class="highlight highlight-source-lua"><pre>local x = 1
if x &lt; 2 then print(x) end</pre></div>
<blockquote><p dir="auto">Quoted <em>tip</em></p></blockquote>
<table><thead><tr><th>Flavor</th><th>Status</th></tr></thead><tbody><tr><td>Retail</td><td>Yes</td></tr></tbody></table>
<ol start="2"><li>two</li><li>three</li></ol><hr>"##;

    #[test]
    fn github_readmes_lose_badges_and_anchors_not_pictures() {
        let bases = Bases {
            link: url::Url::parse("https://github.com/").ok(),
            image: None,
        };
        let doc = to_doc(README, bases);
        assert_eq!(
            plain(&doc),
            [
                "h1 WeakAuras",
                "img https://camo.githubusercontent.com/e488/6874",
                "Install with /wa in game, or see the wiki.",
                "code local x = 1\nif x < 2 then print(x) end",
                "> Quoted tip",
                "Flavor · Status",
                "Retail · Yes",
                "2. two",
                "3. three",
            ]
        );
        let Block::Image(logo) = &doc.blocks[1] else {
            panic!()
        };
        assert_eq!(logo.alt, "Logo");
        let Block::Text { spans, .. } = &doc.blocks[2] else {
            panic!()
        };
        assert!(spans.iter().any(|s| s.code && s.text == "/wa"));
        assert_eq!(
            spans.iter().find_map(|s| s.link.as_deref()),
            Some("https://github.com/WeakAuras/WeakAuras2/wiki")
        );
        assert!(matches!(
            doc.blocks[0],
            Block::Text {
                kind: Kind::Heading(1),
                ..
            }
        ));
    }

    #[test]
    fn broken_html_never_panics_and_keeps_its_text() {
        for junk in [
            "<",
            "a < b and c > d",
            "<p",
            "<a href=\"x",
            "<img src='https://i.imgur.com/a.png' alt=\"unterminated",
            "</li></ul></ol></blockquote></pre>text",
            "<!-- never closed",
            "<!DOCTYPE html><p>ok",
            "&#0; &#xD800; &#99999999999; &unknown; &",
            "<script>never closed",
            "<p a=b c d=\"e\" / >x</p>",
            "<ünïcode>é</ünïcode>",
        ] {
            let _ = to_doc(junk, Bases::default());
        }
        assert_eq!(
            plain(&to_doc("a < b and c > d", Bases::default())),
            ["a < b and c > d"]
        );
        assert_eq!(plain(&to_doc("<!DOCTYPE html><p>ok", Bases::default())), ["ok"]);
        assert_eq!(plain(&to_doc("</li></ul>text", Bases::default())), ["text"]);
    }

    /// The size of each picture of `html`, in order.
    fn sizes(html: &str) -> Vec<(Option<u32>, Option<u32>)> {
        to_doc(html, Bases::default())
            .images()
            .map(|i| (i.width, i.height))
            .collect()
    }

    #[test]
    fn pictures_keep_the_size_they_declare() {
        // CurseForge's editor: both sizes, or only the width (Details!).
        assert_eq!(
            sizes(r#"<p><img src="https://media.forgecdn.net/a.png" width="600" height="300"></p>"#),
            [(Some(600), Some(300))]
        );
        let logo = to_doc(CURSEFORGE, Bases::same("https://www.curseforge.com/"));
        let logo = logo.images().next().unwrap();
        assert_eq!((logo.width, logo.height), (Some(600), None));
        // A README: unquoted, with a unit, upper case.
        assert_eq!(
            sizes(r#"<img src="https://i.imgur.com/a.png" WIDTH=300px height=" 120 ">"#),
            [(Some(300), Some(120))]
        );
    }

    #[test]
    fn sizes_that_say_nothing_usable_are_dropped() {
        for html in [
            r#"<img src="https://i.imgur.com/a.png" width="100%" height="auto">"#,
            r#"<img src="https://i.imgur.com/a.png" width="0" height="0">"#,
            r#"<img src="https://i.imgur.com/a.png" width="4097" height="99999999999999">"#,
            r#"<img src="https://i.imgur.com/a.png" width="-5" height="12.5">"#,
            r#"<img src="https://i.imgur.com/a.png" width="" height="px">"#,
            r#"<img src="https://i.imgur.com/a.png" width="3em" height="+40">"#,
            r#"<img src="https://i.imgur.com/a.png">"#,
        ] {
            assert_eq!(sizes(html), [(None, None)], "{html}");
        }
        assert_eq!(
            sizes(r#"<img src="https://i.imgur.com/a.png" width="4096" height="1">"#),
            [(Some(4096), Some(1))]
        );
    }

    #[test]
    fn entities_decode() {
        assert_eq!(
            decode_entities("Tom &amp; Jerry &#39;s &#x2014; &hellip;"),
            "Tom & Jerry 's — …"
        );
        assert_eq!(decode_entities("&#0; & &nope;"), "&#0; & &nope;");
        assert_eq!(decode_entities("&#xD800;"), "\u{FFFD}");
    }
}
