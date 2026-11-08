//! The text of an addon page, in a form the GUI can draw without parsing
//! anything: a flat list of blocks (headings, paragraphs, list items,
//! quotes, code, pictures, rules), each a run of styled spans.
//!
//! The three converters (HTML for CurseForge and GitHub, BBCode for
//! WoWInterface, Markdown for Tukui) all drive the same [`Builder`], so the
//! limits and the link rules below hold whatever the source.
//!
//! Descriptions are untrusted input: the builder never recurses (nesting is
//! a counter), keeps at most [`MAX_BLOCKS`] blocks, [`MAX_IMAGES`] pictures
//! and [`MAX_CHARS`] characters, and turns every link into an `https://` URL
//! [`crate::http::browser_url`] accepts, or no link at all.

use url::Url;

/// Most blocks kept: a long CurseForge description is about 200. Past it the
/// page says the rest is on the site ([`Doc::truncated`]).
pub const MAX_BLOCKS: usize = 800;

/// Most pictures kept. A README with a screenshot per feature has a dozen.
pub const MAX_IMAGES: usize = 40;

/// Most characters of text kept, all blocks together.
pub const MAX_CHARS: usize = 256 * 1024;

/// Deepest list level drawn; deeper items are drawn at this level.
pub const MAX_DEPTH: u8 = 6;

/// A description, ready to draw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doc {
    /// In reading order.
    pub blocks: Vec<Block>,
    /// Something was left out to stay within the limits above.
    pub truncated: bool,
}

impl Doc {
    /// Whether there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Every picture, in reading order.
    pub fn images(&self) -> impl Iterator<Item = &Image> {
        self.blocks.iter().filter_map(|b| match b {
            Block::Image(i) => Some(i),
            _ => None,
        })
    }
}

/// One block of a [`Doc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Running text of some kind.
    Text {
        /// Paragraph, heading, list item or quote.
        kind: Kind,
        /// Never empty.
        spans: Vec<Span>,
    },
    /// Preformatted text, drawn in the mono font.
    Code(String),
    /// A picture on a line of its own.
    Image(Image),
    /// A horizontal rule.
    Rule,
}

/// What a [`Block::Text`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Plain paragraph.
    Paragraph,
    /// Heading of level 1 (largest) to 6.
    Heading(u8),
    /// A list item at `depth` (1 is the outer list, at most [`MAX_DEPTH`]).
    /// `marker` is `None` for a second paragraph of the same item.
    Item {
        /// Nesting level, from 1.
        depth: u8,
        /// The bullet or number before it.
        marker: Option<Marker>,
    },
    /// Quoted text.
    Quote,
}

/// What starts a list item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// An unordered list's bullet.
    Bullet,
    /// An ordered list's number.
    Number(u32),
}

/// A run of text in one style.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Span {
    /// The text; `\n` is a line break.
    pub text: String,
    /// Bold.
    pub strong: bool,
    /// Italic.
    pub emphasis: bool,
    /// Inline code.
    pub code: bool,
    /// Where it links to: an `https://` URL fit for the browser.
    pub link: Option<String>,
}

/// A picture in a description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Absolute `https://` URL. Untrusted: only [`crate::page::media`] may
    /// fetch it, and only from its allowed hosts.
    pub url: String,
    /// Its alternative text, maybe empty.
    pub alt: String,
    /// The link around it, when there is one.
    pub link: Option<String>,
    /// The width its description declares, in pixels: lets the page keep
    /// the picture's place before it arrives. Only HTML declares one (the
    /// `width` attribute); `None` when absent or not a plain pixel size.
    pub width: Option<u32>,
    /// The height its description declares, in pixels, like `width`.
    pub height: Option<u32>,
}

/// What relative links and pictures are resolved against.
#[derive(Debug, Clone, Default)]
pub(crate) struct Bases {
    pub link: Option<Url>,
    pub image: Option<Url>,
}

impl Bases {
    /// Both resolved against `base`.
    pub fn same(base: &str) -> Self {
        let url = Url::parse(base).ok();
        Self {
            link: url.clone(),
            image: url,
        }
    }
}

/// The link a description's `href` stands for: absolute, `https://` (an
/// `http://` link is upgraded, as old pages are full of them), and accepted
/// by [`crate::http::browser_url`]. Anchors within the page and every other
/// scheme (`mailto:`, `javascript:`) give no link.
pub(crate) fn link_target(raw: &str, base: Option<&Url>) -> Option<String> {
    let url = absolute(raw, base)?;
    crate::http::browser_url(url.as_str())
}

/// The picture a description's `src` stands for: absolute and `https://`
/// like a link. SVG files are left out: there is no decoder for them, and
/// in READMEs they are build badges ("Build Status", "Discord"), not
/// pictures of the addon (WeakAuras2's README opens with three).
pub(crate) fn image_source(raw: &str, base: Option<&Url>) -> Option<String> {
    let url = absolute(raw, base)?;
    if url.path().to_ascii_lowercase().ends_with(".svg") {
        return None;
    }
    crate::http::browser_url(url.as_str())
}

fn absolute(raw: &str, base: Option<&Url>) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') {
        return None;
    }
    let mut url = match Url::parse(raw) {
        Ok(u) => u,
        Err(url::ParseError::RelativeUrlWithoutBase) => base?.join(raw).ok()?,
        Err(_) => return None,
    };
    match url.scheme() {
        "https" => Some(url),
        "http" => url.set_scheme("https").ok().map(|()| url),
        _ => None,
    }
}

/// An open list: whether it counts, the next number, and how many of its
/// items are open (HTML often leaves `<li>` unclosed; BBCode never closes
/// `[*]`), so closing the list closes them too.
#[derive(Debug)]
struct List {
    ordered: bool,
    next: u32,
    open_items: u16,
}

/// Turns a stream of open / close / text events into a [`Doc`]. Nesting is
/// counted, never recursed into, and every counter saturates, so unbalanced
/// tags only end a block early.
#[derive(Debug, Default)]
pub(crate) struct Builder {
    doc: Doc,
    bases: Bases,
    spans: Vec<Span>,
    strong: u16,
    emphasis: u16,
    code: u16,
    links: Vec<Option<String>>,
    lists: Vec<List>,
    /// Items open outside any list (a stray `[*]` or `<li>`).
    loose_items: u16,
    /// The marker of the item just opened, until its first text is out.
    marker: Option<Marker>,
    quote: u16,
    heading: Option<u8>,
    pre: Option<String>,
    skip: u16,
    space: bool,
    images: usize,
    chars: usize,
}

impl Builder {
    pub fn new(bases: Bases) -> Self {
        Self {
            bases,
            ..Self::default()
        }
    }

    /// The document so far, closing whatever is still open.
    pub fn finish(mut self) -> Doc {
        self.close_pre();
        self.flush();
        while matches!(self.doc.blocks.last(), Some(Block::Rule)) {
            self.doc.blocks.pop();
        }
        self.doc
    }

    fn full(&mut self) -> bool {
        if self.doc.blocks.len() >= MAX_BLOCKS || self.chars >= MAX_CHARS {
            self.doc.truncated = true;
            return true;
        }
        false
    }

    /// Text whose whitespace is collapsed, as HTML's is: runs become one
    /// space, and none starts a block or follows a line break.
    pub fn text(&mut self, s: &str) {
        if self.skip > 0 {
            return;
        }
        if self.pre.is_some() {
            return self.raw(s);
        }
        let mut out = String::new();
        for c in s.chars() {
            if c.is_whitespace() {
                self.space = true;
                continue;
            }
            if self.space && (!out.is_empty() || self.ends_in_text()) {
                out.push(' ');
            }
            self.space = false;
            out.push(c);
        }
        // A trailing space goes out now, in this text's style: "Shows your "
        // then "damage" in bold, not "Shows your" and " damage".
        if self.space && (!out.is_empty() || self.ends_in_text()) {
            out.push(' ');
            self.space = false;
        }
        self.push(out);
    }

    /// Text kept exactly as it is (inside `<pre>` and code blocks).
    pub fn raw(&mut self, s: &str) {
        if self.skip > 0 {
            return;
        }
        let Some(pre) = &mut self.pre else {
            return self.push(s.to_string());
        };
        let left = MAX_CHARS.saturating_sub(self.chars);
        let kept: String = s.chars().take(left).collect();
        if kept.len() < s.len() {
            self.doc.truncated = true;
        }
        self.chars += kept.chars().count();
        pre.push_str(&kept);
    }

    /// Inline code, kept as it is.
    pub fn code_span(&mut self, s: &str) {
        if self.skip > 0 || self.pre.is_some() {
            return self.raw(s);
        }
        if self.space && self.ends_in_text() {
            self.push(" ".into());
        }
        self.space = false;
        self.code += 1;
        self.push(s.to_string());
        self.code -= 1;
    }

    /// Whether the current block ends in something a space may follow.
    fn ends_in_text(&self) -> bool {
        self.spans
            .last()
            .and_then(|s| s.text.chars().last())
            .is_some_and(|c| !c.is_whitespace())
    }

    fn push(&mut self, text: String) {
        if text.is_empty() || self.full() {
            return;
        }
        self.chars += text.chars().count();
        let link = self.links.iter().rev().find_map(Clone::clone);
        let (strong, emphasis, code) = (self.strong > 0, self.emphasis > 0, self.code > 0);
        match self.spans.last_mut() {
            Some(last)
                if last.strong == strong
                    && last.emphasis == emphasis
                    && last.code == code
                    && last.link == link =>
            {
                last.text.push_str(&text);
            }
            _ => self.spans.push(Span {
                text,
                strong,
                emphasis,
                code,
                link,
            }),
        }
    }

    /// A line break inside the current block (`<br>`, a BBCode newline).
    pub fn line_break(&mut self) {
        if self.skip > 0 {
            return;
        }
        if let Some(pre) = &mut self.pre {
            pre.push('\n');
            return;
        }
        self.space = false;
        let ends_open = self
            .spans
            .last()
            .and_then(|s| s.text.chars().last())
            .is_some_and(|c| c != '\n');
        if ends_open {
            self.push("\n".into());
        }
    }

    /// End the current block, if it has any text.
    pub fn flush(&mut self) {
        self.space = false;
        let mut spans = std::mem::take(&mut self.spans);
        while let Some(last) = spans.last_mut() {
            let trimmed = last.text.trim_end().len();
            last.text.truncate(trimmed);
            if last.text.is_empty() {
                spans.pop();
            } else {
                break;
            }
        }
        if spans.is_empty() || self.doc.blocks.len() >= MAX_BLOCKS {
            if !spans.is_empty() {
                self.doc.truncated = true;
            }
            return;
        }
        let kind = self.kind();
        self.doc.blocks.push(Block::Text { kind, spans });
    }

    fn kind(&mut self) -> Kind {
        if let Some(level) = self.heading {
            return Kind::Heading(level);
        }
        let in_item = self.loose_items > 0 || self.lists.iter().any(|l| l.open_items > 0);
        if in_item {
            let depth = u8::try_from(self.lists.len())
                .unwrap_or(u8::MAX)
                .clamp(1, MAX_DEPTH);
            return Kind::Item {
                depth,
                marker: self.marker.take(),
            };
        }
        if self.quote > 0 {
            return Kind::Quote;
        }
        Kind::Paragraph
    }

    pub fn strong(&mut self, open: bool) {
        self.strong = step(self.strong, open);
    }

    pub fn emphasis(&mut self, open: bool) {
        self.emphasis = step(self.emphasis, open);
    }

    pub fn code(&mut self, open: bool) {
        self.code = step(self.code, open);
    }

    /// Open a link to `href`; one that gives no [`link_target`] still opens
    /// (as no link), so its close matches.
    pub fn open_link(&mut self, href: &str) {
        let target = link_target(href, self.bases.link.as_ref());
        self.links.push(target);
    }

    pub fn close_link(&mut self) {
        self.links.pop();
    }

    pub fn open_heading(&mut self, level: u8) {
        self.flush();
        self.heading = Some(level.clamp(1, 6));
    }

    pub fn close_heading(&mut self) {
        self.flush();
        self.heading = None;
    }

    pub fn open_quote(&mut self) {
        self.flush();
        self.quote = step(self.quote, true);
    }

    pub fn close_quote(&mut self) {
        self.flush();
        self.quote = step(self.quote, false);
    }

    pub fn open_list(&mut self, ordered: bool, start: u32) {
        self.flush();
        if self.lists.len() < usize::from(u8::MAX) {
            self.lists.push(List {
                ordered,
                next: start,
                open_items: 0,
            });
        }
    }

    pub fn close_list(&mut self) {
        self.flush();
        self.lists.pop();
        self.marker = None;
    }

    pub fn open_item(&mut self) {
        self.flush();
        let marker = match self.lists.last_mut() {
            Some(list) => {
                list.open_items = step(list.open_items, true);
                if list.ordered {
                    let n = list.next;
                    list.next = list.next.saturating_add(1);
                    Marker::Number(n)
                } else {
                    Marker::Bullet
                }
            }
            None => {
                self.loose_items = step(self.loose_items, true);
                Marker::Bullet
            }
        };
        self.marker = Some(marker);
    }

    pub fn close_item(&mut self) {
        self.flush();
        match self.lists.last_mut() {
            Some(list) => list.open_items = step(list.open_items, false),
            None => self.loose_items = step(self.loose_items, false),
        }
        self.marker = None;
    }

    pub fn open_pre(&mut self) {
        self.flush();
        if self.pre.is_none() {
            self.pre = Some(String::new());
        }
    }

    pub fn close_pre(&mut self) {
        let Some(code) = self.pre.take() else {
            return;
        };
        let code = code.strip_prefix('\n').unwrap_or(&code).trim_end();
        if code.trim().is_empty() || self.full() {
            return;
        }
        self.doc.blocks.push(Block::Code(code.to_string()));
    }

    /// A picture at `src` (see [`image_source`]); left out when it has no
    /// usable address, or past [`MAX_IMAGES`].
    pub fn image(&mut self, src: &str, alt: &str) {
        self.sized_image(src, alt, None, None);
    }

    /// [`Builder::image`] with the size its description declares, already
    /// checked (see [`Image::width`]).
    pub fn sized_image(&mut self, src: &str, alt: &str, width: Option<u32>, height: Option<u32>) {
        if self.skip > 0 || self.pre.is_some() {
            return;
        }
        let Some(url) = image_source(src, self.bases.image.as_ref()) else {
            return;
        };
        if self.images >= MAX_IMAGES {
            self.doc.truncated = true;
            return;
        }
        self.flush();
        if self.full() {
            return;
        }
        self.images += 1;
        let link = self.links.iter().rev().find_map(Clone::clone);
        let alt = alt.split_whitespace().collect::<Vec<_>>().join(" ");
        self.doc.blocks.push(Block::Image(Image {
            url,
            alt,
            link,
            width,
            height,
        }));
    }

    /// A horizontal rule; never two in a row, nor one first.
    pub fn rule(&mut self) {
        if self.skip > 0 {
            return;
        }
        self.flush();
        if matches!(self.doc.blocks.last(), None | Some(Block::Rule)) || self.full() {
            return;
        }
        self.doc.blocks.push(Block::Rule);
    }

    /// Start or stop ignoring everything (`<script>`, `<svg>`).
    pub fn skip(&mut self, open: bool) {
        self.skip = step(self.skip, open);
    }
}

fn step(n: u16, open: bool) -> u16 {
    if open {
        n.saturating_add(1)
    } else {
        n.saturating_sub(1)
    }
}

/// The first line of `doc` that reads as a sentence, as plain text: the
/// one-line summary a WoWInterface file has none of, taken from its
/// description. Lines of one or two words are titles, not summaries:
/// CTMod's description opens with "CTMod" alone.
pub fn first_paragraph(doc: &Doc) -> Option<String> {
    doc.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Text {
                kind: Kind::Paragraph,
                spans,
            } => Some(spans.iter().map(|s| s.text.as_str()).collect::<String>()),
            _ => None,
        })
        .flat_map(|text| text.lines().map(|l| l.trim().to_string()).collect::<Vec<_>>())
        .find(|line| line.split_whitespace().count() >= 3)
}

#[cfg(test)]
pub(crate) fn plain(doc: &Doc) -> Vec<String> {
    doc.blocks
        .iter()
        .map(|b| match b {
            Block::Text { kind, spans } => {
                let text: String = spans.iter().map(|s| s.text.as_str()).collect();
                match kind {
                    Kind::Paragraph => text,
                    Kind::Heading(l) => format!("h{l} {text}"),
                    Kind::Quote => format!("> {text}"),
                    Kind::Item { depth, marker } => {
                        let m = match marker {
                            Some(Marker::Bullet) => "*".to_string(),
                            Some(Marker::Number(n)) => format!("{n}."),
                            None => "+".to_string(),
                        };
                        format!("{}{m} {text}", "  ".repeat(usize::from(*depth) - 1))
                    }
                }
            }
            Block::Code(c) => format!("code {c}"),
            Block::Image(i) => format!("img {}", i.url),
            Block::Rule => "---".to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Option<Url> {
        Url::parse("https://github.com/WeakAuras/WeakAuras2/blob/HEAD/").ok()
    }

    #[test]
    fn links_are_absolute_https_or_nothing() {
        let b = base();
        assert_eq!(
            link_target("docs/README.md", b.as_ref()).as_deref(),
            Some("https://github.com/WeakAuras/WeakAuras2/blob/HEAD/docs/README.md")
        );
        // Old CurseForge descriptions are full of http:// links.
        assert_eq!(
            link_target("http://www.wowace.com/projects/ace3", None).as_deref(),
            Some("https://www.wowace.com/projects/ace3")
        );
        assert_eq!(link_target("#download", b.as_ref()), None);
        assert_eq!(link_target("mailto:a@b.c", b.as_ref()), None);
        assert_eq!(link_target("javascript:alert(1)", b.as_ref()), None);
        assert_eq!(link_target("https://github.com@evil.example/", None), None);
        assert_eq!(link_target("relative/without/base", None), None);
    }

    #[test]
    fn svg_badges_are_not_pictures() {
        let b = base();
        assert_eq!(
            image_source(
                "https://github.com/WeakAuras/WeakAuras2/workflows/CI/badge.svg",
                b.as_ref()
            ),
            None
        );
        assert_eq!(
            image_source("https://i.imgur.com/wwbxeCG.jpeg", None).as_deref(),
            Some("https://i.imgur.com/wwbxeCG.jpeg")
        );
        assert_eq!(image_source("data:image/png;base64,AAAA", None), None);
    }

    #[test]
    fn whitespace_collapses_and_styles_split_spans() {
        let mut b = Builder::new(Bases::default());
        b.text("  Shows   your ");
        b.strong(true);
        b.text("damage");
        b.strong(false);
        b.text(" and\n healing.  ");
        b.flush();
        let doc = b.finish();
        let Block::Text { spans, .. } = &doc.blocks[0] else {
            panic!("{doc:?}")
        };
        let texts: Vec<_> = spans.iter().map(|s| (s.text.as_str(), s.strong)).collect();
        assert_eq!(
            texts,
            [("Shows your ", false), ("damage", true), (" and healing.", false)]
        );
    }

    #[test]
    fn unclosed_items_close_with_their_list() {
        let mut b = Builder::new(Bases::default());
        b.open_list(true, 3);
        b.open_item();
        b.text("three");
        b.open_item();
        b.text("four");
        b.open_list(false, 1);
        b.open_item();
        b.text("inner");
        b.close_list();
        b.close_list();
        b.text("after");
        assert_eq!(plain(&b.finish()), ["3. three", "4. four", "  * inner", "after"]);
    }

    #[test]
    fn limits_hold() {
        let mut b = Builder::new(Bases::default());
        for i in 0..MAX_BLOCKS + 10 {
            b.text(&format!("p{i}"));
            b.flush();
        }
        for _ in 0..MAX_IMAGES + 5 {
            b.image("https://media.forgecdn.net/a.png", "");
        }
        let doc = b.finish();
        assert_eq!(doc.blocks.len(), MAX_BLOCKS);
        assert!(doc.truncated);
    }

    #[test]
    fn the_first_paragraph_is_the_summary() {
        let mut b = Builder::new(Bases::default());
        b.open_heading(2);
        b.text("About");
        b.close_heading();
        b.text("The ultimate encounter helper.");
        b.line_break();
        b.text("Second line.");
        let doc = b.finish();
        assert_eq!(
            first_paragraph(&doc).as_deref(),
            Some("The ultimate encounter helper.")
        );

        let mut b = Builder::new(Bases::default());
        b.text("CTMod");
        b.line_break();
        b.text("CTMod is one of the longest-running collections of addons.");
        let doc = b.finish();
        assert_eq!(
            first_paragraph(&doc).as_deref(),
            Some("CTMod is one of the longest-running collections of addons.")
        );
    }
}
