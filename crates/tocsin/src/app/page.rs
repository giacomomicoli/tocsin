//! The addon page: what an addon's site says about it (summary, gallery,
//! description), opened from a Library row or a Get addons result, with a
//! way back to that list where it was left.
//!
//! The page and its pictures come from the core (`tocsin_core::page`): the
//! description already parsed, the pictures decoded and scaled. This module
//! keeps them in state, builds the text spans once per page, and draws.

use std::collections::HashMap;

use iced::futures::stream::{self, StreamExt};
use iced::widget::scrollable::RelativeOffset;
use iced::widget::{button, column, container, image, operation, rich_text, row, space, span, text, Column};
use iced::{Alignment, ContentFit, Element, Fill, Font, Length, Padding, Task};

use tocsin_core::page::{self, media, Block, Image, Kind, Marker, Page, PageError};
use tocsin_core::SourceRef;

use super::widgets::{self, badge, icon_slot, monogram};
use super::{
    AddonView, App, Fit, Message, PageState, PictureState, Screen, Shown, DISCOVER_LIST, LIBRARY_LIST,
    PAGE_PICTURE_BYTES, PAGE_SCROLL, PICTURE_CONCURRENCY,
};
use crate::theme::{self, size, space as sp, Tokens, Tone};

/// A text span of a page, made once.
type PageSpan = iced::widget::text::Span<'static, String, Font>;

impl App {
    /// The nav item lit: the page's own list while an addon page is open.
    pub(super) fn nav_screen(&self) -> Screen {
        match (&self.screen, &self.addon) {
            (Screen::Addon, Some(v)) => v.back,
            (screen, _) => *screen,
        }
    }

    pub(super) fn open_library_page(&mut self, folder: String) -> Task<Message> {
        let Some(a) = self.addons.iter().find(|a| a.folder == folder) else {
            return Task::none();
        };
        let creds = self.creds.as_deref().cloned().unwrap_or_default();
        let sources = page::page_sources(&a.sources, &creds);
        let view = AddonView {
            name: a.display_name.clone(),
            icon: self.library_icons.get(&folder).cloned(),
            found: None,
            ..AddonView::new(Screen::Library, sources)
        };
        self.open_view(view)
    }

    pub(super) fn open_found_page(&mut self, seq: u64, index: usize) -> Task<Message> {
        if seq != self.discover.seq {
            return Task::none();
        }
        let Some(f) = self.discover.outcome.results.get(index) else {
            return Task::none();
        };
        let creds = self.creds.as_deref().cloned().unwrap_or_default();
        let sources = page::page_sources(f.hits.iter().map(|h| &h.source), &creds);
        let view = AddonView {
            name: f.primary().name.clone(),
            icon: f.icon().map(str::to_string),
            found: Some((seq, index)),
            ..AddonView::new(Screen::Discover, sources)
        };
        self.open_view(view)
    }

    fn open_view(&mut self, view: AddonView) -> Task<Message> {
        if view.sources.is_empty() {
            return Task::none();
        }
        self.addon = Some(view);
        self.screen = Screen::Addon;
        self.notice = None;
        self.request_page()
    }

    /// Ask for the page of the source shown, forgetting the previous one.
    fn request_page(&mut self) -> Task<Message> {
        self.page_seq += 1;
        let seq = self.page_seq;
        let Some(v) = self.addon.as_mut() else {
            return Task::none();
        };
        v.reset(seq);
        let source = v.sources[v.shown].clone();
        let http = self.http.clone();
        let creds = self.creds.clone().unwrap_or_default();
        let fetch = Task::perform(
            async move {
                page::fetch(&http, &creds, &source)
                    .await
                    .map(Box::new)
                    .map_err(|e| (e.to_string(), matches!(e, PageError::NeedsKey)))
            },
            move |result| Message::PageLoaded(seq, result),
        );
        Task::batch([fetch, operation::snap_to(PAGE_SCROLL, RelativeOffset::START)])
    }

    pub(super) fn show_page_source(&mut self, i: usize) -> Task<Message> {
        match self.addon.as_mut() {
            Some(v) if i < v.sources.len() && i != v.shown => {
                v.shown = i;
                self.request_page()
            }
            _ => Task::none(),
        }
    }

    pub(super) fn page_loaded(
        &mut self,
        seq: u64,
        result: Result<Box<Page>, (String, bool)>,
    ) -> Task<Message> {
        let tokens = theme::tokens_for(self.dark);
        let Some(v) = self.addon.as_mut().filter(|v| v.seq == seq) else {
            return Task::none();
        };
        let page = match result {
            Ok(page) => *page,
            Err((why, key)) => {
                v.state = PageState::Failed(why, key);
                return Task::none();
            }
        };
        // In the order the eye meets them: the screenshot shown large, the
        // thumbnails under it, then the description's pictures in order.
        let mut wanted: Vec<(Fit, String)> = page
            .gallery
            .first()
            .map(|s| (Fit::Full, s.url.clone()))
            .into_iter()
            .collect();
        wanted.extend(page.gallery.iter().map(|s| (Fit::Thumb, s.thumb.clone())));
        wanted.extend(page.body.images().map(|i| (Fit::Full, i.url.clone())));
        let spans = spans_of(&page, tokens);
        v.state = PageState::Ready(Box::new(Shown { page, spans }));
        self.fetch_pictures(wanted)
    }

    /// Start downloading those of `wanted` not asked for yet,
    /// [`PICTURE_CONCURRENCY`] at a time. A picture on a host pages may not
    /// fetch from is marked unavailable right away: the page links to it.
    fn fetch_pictures(&mut self, wanted: Vec<(Fit, String)>) -> Task<Message> {
        let Some(v) = self.addon.as_mut() else {
            return Task::none();
        };
        let seq = v.seq;
        let mut fetch = Vec::new();
        for (fit, url) in wanted {
            let map = v.map(fit);
            if map.contains_key(&url) {
                continue;
            }
            if media::is_allowed(&url) {
                map.insert(url.clone(), PictureState::Loading);
                fetch.push((fit, url));
            } else {
                map.insert(url, PictureState::Unavailable);
            }
        }
        if fetch.is_empty() {
            return Task::none();
        }
        let http = self.http.clone();
        let downloads = stream::iter(fetch)
            .map(move |(fit, url)| {
                let http = http.clone();
                async move {
                    let (w, h) = fit.bounds();
                    let picture = media::fetch(&http, &url, w, h).await.map_err(|e| e.to_string());
                    (fit, url, picture)
                }
            })
            .buffer_unordered(PICTURE_CONCURRENCY);
        let (task, handle) = Task::run(downloads, move |(fit, url, picture)| {
            Message::PictureLoaded(seq, fit, url, picture)
        })
        .abortable();
        v.batch.push(handle.abort_on_drop());
        task
    }

    pub(super) fn picture_loaded(
        &mut self,
        seq: u64,
        fit: Fit,
        url: String,
        result: Result<media::Picture, String>,
    ) {
        let Some(v) = self.addon.as_mut().filter(|v| v.seq == seq) else {
            return;
        };
        let budget_left = PAGE_PICTURE_BYTES.saturating_sub(v.picture_bytes);
        let state = match result {
            Ok(p) if p.rgba.len() <= budget_left => {
                v.picture_bytes += p.rgba.len();
                PictureState::Ready(image::Handle::from_rgba(p.width, p.height, p.rgba))
            }
            Ok(_) => PictureState::Unavailable,
            Err(e) => {
                // The URL is a public address from an API answer.
                tracing::debug!(%url, error = %e, "no picture for an addon page");
                PictureState::Unavailable
            }
        };
        if let Some(slot) = v.map(fit).get_mut(&url) {
            *slot = state;
        }
    }

    pub(super) fn show_shot(&mut self, i: usize) -> Task<Message> {
        let Some(v) = self.addon.as_mut() else {
            return Task::none();
        };
        let PageState::Ready(shown) = &v.state else {
            return Task::none();
        };
        let Some(url) = shown.page.gallery.get(i).map(|s| s.url.clone()) else {
            return Task::none();
        };
        v.shot = i;
        self.fetch_pictures(vec![(Fit::Full, url)])
    }

    /// Back to the list, scrolled where it was left.
    pub(super) fn page_back(&mut self) -> Task<Message> {
        let back = self.addon.take().map_or(Screen::Library, |v| v.back);
        self.screen = back;
        self.notice = None;
        match back {
            Screen::Discover => operation::scroll_to(DISCOVER_LIST, self.discover_offset),
            _ => operation::scroll_to(LIBRARY_LIST, self.library_offset),
        }
    }

    // ------------------------------------------------------------------ view

    pub(super) fn addon_page(&self) -> Element<'_, Message> {
        let Some(v) = &self.addon else {
            return space::vertical().height(0).into();
        };
        let source = &v.sources[v.shown];
        let page = match &v.state {
            PageState::Ready(s) => Some(&s.page),
            _ => None,
        };
        let name = page
            .map(|p| p.name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or(&v.name);
        let subtitle = match page.map(|p| p.authors.join(", ")).filter(|a| !a.is_empty()) {
            Some(authors) => format!("{} · by {authors}", source.provider()),
            None => source.provider().to_string(),
        };
        let back = match v.back {
            Screen::Discover => "Back to Get addons",
            _ => "Back to Library",
        };
        let url = page.map_or_else(|| source.web_url(), |p| p.url.clone());
        let mut actions = row![
            widgets::ghost(back, size::META, sp::BTN_SM).on_press(Message::PageBack),
            widgets::ghost("Open", size::META, sp::BTN_SM)
                .on_press_maybe(url.starts_with("https://").then_some(Message::OpenUrl(url))),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if let Some(install) = self.page_install(v, source) {
            actions = actions.push(install);
        }

        let mut col = column![widgets::page_head(name, subtitle, Some(actions.into()))].spacing(sp::PAGE_GAP);
        // One chip per site with a page: what each says can differ.
        if v.sources.len() > 1 {
            let chips = v.sources.iter().enumerate().fold(row![].spacing(8), |r, (i, s)| {
                r.push(widgets::chip(s.provider(), i == v.shown).on_press(Message::PageSource(i)))
            });
            col = col.push(chips);
        }
        let body: Element<'_, Message> = match &v.state {
            PageState::Loading => container(self.loader("Loading the page…")).center(Fill).into(),
            PageState::Failed(why, key) => {
                let mut c = column![text(why).size(size::CONTROL).style(theme::muted)]
                    .spacing(10)
                    .align_x(Alignment::Center);
                if *key {
                    c = c.push(
                        widgets::ghost("Add a key", size::META, sp::BTN_SM)
                            .on_press(Message::Show(Screen::Settings)),
                    );
                }
                container(c).center(Fill).into()
            }
            PageState::Ready(shown) => widgets::scroll(
                container(self.page_body(v, shown))
                    .max_width(sp::READ_W)
                    .width(Fill),
            )
            .id(PAGE_SCROLL)
            .into(),
        };
        col.push(body).into()
    }

    /// Install from the source shown, for a page opened from a search
    /// result: the same rules as the result's own line.
    fn page_install<'a>(&self, v: &AddonView, source: &SourceRef) -> Option<Element<'a, Message>> {
        let (seq, index) = v.found?;
        if seq != self.discover.seq {
            return None;
        }
        let f = self.discover.outcome.results.get(index)?;
        if self.installed.contains(f) {
            return Some(badge("Installed", Tone::Ok));
        }
        let hit = f.hits.iter().find(|h| &h.source == source)?;
        if hit.site_only || self.discover.site_only.contains(&hit.source) {
            return Some(badge("Site only", Tone::Manual));
        }
        let can_install = !self.busy && !self.asking() && self.active_dir().is_some();
        let running = self.discover.installing.as_ref() == Some(source);
        let label = if running { "Installing…" } else { "Install" };
        Some(
            widgets::primary(label, size::META, sp::BTN_SM)
                .on_press_maybe(can_install.then(|| Message::InstallFrom(source.clone(), hit.name.clone())))
                .into(),
        )
    }

    fn page_body<'a>(&'a self, v: &'a AddonView, shown: &'a Shown) -> Element<'a, Message> {
        let p = &shown.page;
        let mut intro = row![icon_slot(
            monogram(&v.name),
            false,
            false,
            self.icon(v.icon.as_deref())
        )]
        .spacing(14)
        .align_y(Alignment::Center);
        if !p.summary.is_empty() {
            intro = intro.push(text(&p.summary).size(size::ROW).style(theme::body).width(Fill));
        }
        let mut col = Column::new().spacing(sp::PROSE_GAP).width(Fill).push(intro);
        if !p.gallery.is_empty() {
            col = col.push(self.gallery(v, p));
        }
        if p.body.is_empty() {
            col = col.push(
                text("The site gives no description for it.")
                    .size(size::CONTROL)
                    .style(theme::muted),
            );
        }
        for (block, spans) in p.body.blocks.iter().zip(&shown.spans) {
            col = col.push(self.block(v, block, spans));
        }
        if p.body.truncated {
            col = col.push(
                row![
                    text("The rest is on the site.")
                        .size(size::CONTROL)
                        .style(theme::muted)
                        .width(Fill),
                    widgets::ghost("Open", size::META, sp::BTN_SM).on_press(Message::OpenUrl(p.url.clone())),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        col.into()
    }

    /// The screenshot shown large, its caption, and every thumbnail.
    fn gallery<'a>(&'a self, v: &'a AddonView, p: &'a Page) -> Element<'a, Message> {
        let i = v.shot.min(p.gallery.len() - 1);
        let shot = &p.gallery[i];
        let large: Element<'a, Message> = match v.pictures.get(&shot.url) {
            Some(PictureState::Ready(handle)) => button(
                image(handle.clone())
                    .content_fit(ContentFit::Contain)
                    .width(Fill)
                    .height(sp::SHOT_H),
            )
            .padding(Padding::ZERO)
            .style(theme::name_link(false))
            .on_press(Message::OpenUrl(shot.url.clone()))
            .into(),
            Some(PictureState::Unavailable) => self.picture_link(&shot.title, &shot.url, None),
            _ => self.loader("Loading the picture…"),
        };
        let large = container(large)
            .center_x(Fill)
            .center_y(sp::SHOT_H)
            .style(theme::sunken);
        let mut col = column![large].spacing(sp::ROW_GAP);
        if !shot.title.is_empty() {
            col = col.push(text(&shot.title).size(size::META).style(theme::muted));
        }
        if p.gallery.len() > 1 {
            let thumbs = p
                .gallery
                .iter()
                .enumerate()
                .fold(row![].spacing(sp::ROW_GAP), |r, (j, s)| {
                    let inner: Element<'a, Message> = match v.thumbs.get(&s.thumb) {
                        Some(PictureState::Ready(handle)) => image(handle.clone()).height(sp::THUMB_H).into(),
                        _ => space::horizontal()
                            .width(sp::THUMB_H * 16.0 / 9.0)
                            .height(sp::THUMB_H)
                            .into(),
                    };
                    r.push(
                        button(inner)
                            .padding(sp::THUMB_PAD)
                            .style(theme::thumb(j == i))
                            .on_press(Message::PageShot(j)),
                    )
                });
            col = col.push(thumbs.wrap().vertical_spacing(sp::ROW_GAP));
        }
        col.into()
    }

    fn block<'a>(
        &'a self,
        v: &'a AddonView,
        block: &'a Block,
        spans: &'a [PageSpan],
    ) -> Element<'a, Message> {
        match block {
            Block::Text { kind, .. } => text_block(*kind, spans),
            Block::Code(code) => container(text(code).size(size::META).font(theme::MONO))
                .padding(sp::TIP_PAD)
                .width(Fill)
                .style(theme::sunken)
                .into(),
            Block::Image(img) => match v.pictures.get(&img.url) {
                Some(PictureState::Ready(handle)) => {
                    let picture = image(handle.clone()).content_fit(ContentFit::ScaleDown);
                    match &img.link {
                        Some(link) => button(picture)
                            .padding(Padding::ZERO)
                            .style(theme::name_link(false))
                            .on_press(Message::OpenUrl(link.clone()))
                            .into(),
                        None => picture.into(),
                    }
                }
                Some(PictureState::Unavailable) => self.picture_link(&img.alt, &img.url, img.link.as_deref()),
                _ => self.picture_placeholder(img),
            },
            Block::Rule => widgets::rule(),
        }
    }

    /// The place a description picture keeps while it loads, so that the
    /// text under it does not move when it arrives: a sunken box with the
    /// loader, as wide as the column and as tall as [`placeholder_height`].
    /// Clipped: a picture declared a few pixels tall keeps those few.
    fn picture_placeholder<'a>(&self, img: &Image) -> Element<'a, Message> {
        container(self.loader("Loading the picture…"))
            .center_x(Fill)
            .center_y(placeholder_height(img))
            .clip(true)
            .style(theme::sunken)
            .into()
    }

    /// A picture that is not shown: what it is, where, and Open (its link,
    /// else the picture itself).
    fn picture_link<'a>(&self, alt: &str, url: &str, link: Option<&str>) -> Element<'a, Message> {
        let what = if alt.is_empty() { "Picture" } else { alt };
        let label = match media::host_of(url) {
            Some(host) => format!("{what} · {host}"),
            None => what.to_string(),
        };
        let target = link.unwrap_or(url).to_string();
        container(
            row![
                container(widgets::one_line(
                    text(label).size(size::META).style(theme::muted)
                ))
                .width(Fill)
                .clip(true),
                widgets::ghost("Open", size::META, sp::BTN_SM).on_press(Message::OpenUrl(target)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding(sp::TIP_PAD)
        .width(Fill)
        .style(theme::sunken)
        .into()
    }
}

impl AddonView {
    fn new(back: Screen, sources: Vec<SourceRef>) -> Self {
        Self {
            back,
            name: String::new(),
            icon: None,
            sources,
            shown: 0,
            found: None,
            seq: 0,
            state: PageState::Loading,
            shot: 0,
            thumbs: HashMap::new(),
            pictures: HashMap::new(),
            picture_bytes: 0,
            batch: Vec::new(),
        }
    }

    /// Back to loading, for page number `seq`: pictures and their
    /// downloads go.
    fn reset(&mut self, seq: u64) {
        self.seq = seq;
        self.state = PageState::Loading;
        self.shot = 0;
        self.thumbs.clear();
        self.pictures.clear();
        self.picture_bytes = 0;
        self.batch.clear();
    }

    /// Whether the page or one of its pictures is still on its way: its
    /// loaders are on screen and need the timer.
    pub(super) fn is_loading(&self) -> bool {
        matches!(self.state, PageState::Loading)
            || self
                .thumbs
                .values()
                .chain(self.pictures.values())
                .any(|p| matches!(p, PictureState::Loading))
    }

    fn map(&mut self, fit: Fit) -> &mut HashMap<String, PictureState> {
        match fit {
            Fit::Thumb => &mut self.thumbs,
            Fit::Full => &mut self.pictures,
        }
    }
}

/// How tall a description picture is drawn, to keep its place while it
/// loads: from the size its description declares, decoded to fit
/// [`Fit::Full`] and scaled down to the reading column at its widest
/// ([`sp::READ_W`]); [`sp::PICTURE_H`] when it declares no size. In a window
/// narrow enough to make the column narrower, the picture comes out a
/// little shorter than its place.
fn placeholder_height(img: &Image) -> f32 {
    let (max_w, max_h) = Fit::Full.bounds();
    match (img.width, img.height) {
        (Some(w), Some(h)) => media::drawn_height(w, h, max_w, max_h, sp::READ_W as u32) as f32,
        _ => sp::PICTURE_H,
    }
}

/// A heading, paragraph, quote or list item.
fn text_block<'a>(kind: Kind, spans: &'a [PageSpan]) -> Element<'a, Message> {
    // A span of the page lives as long as the page: borrowed for the frame.
    let spans: &'a [iced::widget::text::Span<'a, String, Font>] = spans;
    let rich = rich_text(spans).on_link_click(Message::OpenUrl);
    match kind {
        Kind::Heading(level) => {
            let (size, style): (f32, fn(&iced::Theme) -> text::Style) = if level <= 2 {
                (size::SECTION, theme::title)
            } else {
                (size::ROW, theme::gold)
            };
            container(rich.size(size).style(style))
                .padding(Padding::ZERO.top(sp::PROSE_GAP as f32))
                .into()
        }
        Kind::Paragraph => rich.size(size::TEXT).style(theme::body).into(),
        Kind::Quote => container(rich.size(size::TEXT).style(theme::muted))
            .padding(sp::TIP_PAD)
            .width(Fill)
            .style(theme::row_child)
            .into(),
        Kind::Item { depth, marker } => {
            let mark: Element<'a, Message> = match marker {
                Some(Marker::Bullet) => text("◆").size(size::XS).style(theme::gold).into(),
                Some(Marker::Number(n)) => text(format!("{n}.")).size(size::TEXT).style(theme::muted).into(),
                None => space::horizontal().width(0).into(),
            };
            let indent = sp::CHILD_INDENT * f32::from(depth.saturating_sub(1));
            row![
                space::horizontal().width(indent),
                container(mark).width(Length::Fixed(sp::CHILD_INDENT)),
                rich.size(size::TEXT).style(theme::body).width(Fill),
            ]
            .align_y(Alignment::Start)
            .into()
        }
    }
}

/// The spans of every text block of `page`, styled for `tokens`: bold in
/// the medium face (the UI font has no bold), code in mono, headings in the
/// display font, links in the theme's gold and underlined. The UI font has
/// no italic either: emphasis reads as plain text.
fn spans_of(page: &Page, tokens: &Tokens) -> Vec<Vec<PageSpan>> {
    page.body
        .blocks
        .iter()
        .map(|b| match b {
            Block::Text { kind, spans } => {
                let heading = matches!(kind, Kind::Heading(_));
                spans
                    .iter()
                    .map(|s| {
                        let font = if s.code {
                            theme::MONO
                        } else if heading {
                            theme::DISPLAY
                        } else if s.strong {
                            theme::UI_MEDIUM
                        } else {
                            theme::UI
                        };
                        let mut out: PageSpan = span(s.text.clone()).font(font);
                        // JetBrains Mono runs larger than Alegreya Sans at
                        // the same size: a step down keeps the line even.
                        if s.code && !heading {
                            out = out.size(size::META);
                        }
                        match &s.link {
                            Some(link) => out.link(link.clone()).color(tokens.text_gold).underline(true),
                            None => out,
                        }
                    })
                    .collect()
            }
            _ => Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: Option<u32>, height: Option<u32>) -> Image {
        Image {
            url: "https://media.forgecdn.net/attachments/description/61284/banner.png".into(),
            alt: String::new(),
            link: None,
            width,
            height,
        }
    }

    #[test]
    fn a_loading_picture_keeps_the_place_it_will_take() {
        // CurseForge's `<img width="600" height="300">`: as declared.
        assert_eq!(placeholder_height(&picture(Some(600), Some(300))), 300.0);
        // A full HD screenshot: decoded to 1280 wide, drawn 840 wide.
        assert_eq!(placeholder_height(&picture(Some(1920), Some(1080))), 473.0);
        // No size, or half of one (`width="600"` alone): the token.
        assert_eq!(placeholder_height(&picture(None, None)), sp::PICTURE_H);
        assert_eq!(placeholder_height(&picture(Some(600), None)), sp::PICTURE_H);
    }
}
