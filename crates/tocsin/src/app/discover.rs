//! The "Get addons" page: search across providers, one row per addon with
//! a line per site offering it, and the "replace these folders?" card.

use iced::widget::{button, column, container, row, space, text, text_input, Column};
use iced::{Alignment, Element, Fill, Length, Padding};

use tocsin_core::addon::Compat;
use tocsin_core::disk::human_size;
use tocsin_core::search::{Found, Hit};
use tocsin_core::secrets::Secret;
use tocsin_core::SourceRef;

use super::widgets::{self, badge, ellipsize, icon_slot, monogram};
use super::{App, CatalogState, Message, PendingInstall, ResolvedInstall, Screen, DISCOVER_LIST};
use crate::theme::{self, size, Tone};

impl App {
    pub(super) fn discover(&self) -> Element<'_, Message> {
        let d = &self.discover;
        let target = match self.active_dir() {
            Some(dir) if self.client.is_empty() => format!("Installs into {}", dir.display()),
            Some(dir) => format!("Installs into {} · {}", self.client, dir.display()),
            None => "Add a game folder in Settings to install addons.".to_string(),
        };
        let placeholder = format!("Search addons for {}…", self.flavor.label());
        let bar = row![
            text_input(&placeholder, &d.query)
                .on_input(Message::DiscoverQuery)
                .on_submit(Message::DiscoverSearch)
                .padding(10)
                .size(size::CONTROL)
                .width(Fill)
                .style(theme::input),
            widgets::primary(
                if d.searching { "Searching…" } else { "Search" },
                size::CONTROL,
                [9, 16]
            )
            .on_press_maybe((!d.searching).then_some(Message::DiscoverSearch)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let mut controls = column![bar, self.catalog_status()].spacing(8);
        // Without a key, search still covers WoWInterface, Tukui and GitHub.
        if !self.has_key(Secret::CurseForge) {
            controls = controls.push(hint_row(
                "CurseForge needs an API key to be searched too.",
                false,
                "Add a key",
            ));
        }
        if self.github_token_refused() {
            controls = controls.push(hint_row(
                "GitHub refused your token: searching it without one.",
                true,
                "Fix in Settings",
            ));
        }
        for e in &d.outcome.errors {
            controls = controls.push(text(e).size(size::META).style(theme::tone(Tone::Update)));
        }

        let mut page =
            column![widgets::page_head("Get Addons", target, None), controls].spacing(theme::space::PAGE_GAP);
        if let Some(p) = &d.confirm {
            page = page.push(self.confirm_card(p));
        }
        if let Some(r) = &d.confirm_large {
            page = page.push(self.large_card(r));
        }

        if d.searching {
            let waiting = container(self.loader("Searching…")).center(Fill);
            return page.push(waiting).into();
        }
        if d.outcome.results.is_empty() {
            let hint = match &d.searched {
                Some(q) => format!("Nothing found for “{q}”."),
                None => "Find addons you don't have yet. An addon offered by several sites is listed once: \
                         you pick where to install it from."
                    .to_string(),
            };
            let empty = container(text(hint).size(size::CONTROL).style(theme::muted)).center(Fill);
            return page.push(empty).into();
        }
        page = page.push(self.results_bar());
        // Strict hides what no site confirms, but never an addon already
        // installed: the row still says whether that copy fits this game.
        let shown: Vec<(usize, &Found)> = d
            .outcome
            .results
            .iter()
            .enumerate()
            .filter(|(_, f)| !d.strict || f.confirmed() || self.installed.contains(f))
            .collect();
        if shown.is_empty() {
            let hint = format!(
                "No site confirms these results for {}. Everything found shows them all.",
                self.flavor.label()
            );
            let empty = container(text(hint).size(size::CONTROL).style(theme::muted)).center(Fill);
            return page.push(empty).into();
        }
        let list = shown
            .into_iter()
            .fold(Column::new().spacing(theme::space::ROW_GAP), |col, (i, f)| {
                col.push(self.found_row(i, f))
            });
        // Its position is kept for the addon page's Back.
        let list = widgets::scroll(list)
            .id(DISCOVER_LIST)
            .on_scroll(|v| Message::ListScrolled(Screen::Discover, v.absolute_offset()));
        page.push(list).into()
    }

    /// "Results for Forever 1.60.1 · 9 confirmed · 4 unverified", with the
    /// two chips that show only the confirmed ones or everything found.
    /// With the game's build unknown a site's listing for the flavor still
    /// confirms a result, so the line and the chips are the same.
    fn results_bar(&self) -> Element<'_, Message> {
        let results = &self.discover.outcome.results;
        let confirmed = results.iter().filter(|f| f.confirmed()).count();
        let client = if self.client.is_empty() {
            self.flavor.label()
        } else {
            self.client.as_str()
        };
        let summary = format!(
            "Results for {client} · {confirmed} confirmed · {} unverified",
            results.len() - confirmed
        );
        let strict = self.discover.strict;
        row![
            text(summary).size(size::META).style(theme::muted).width(Fill),
            widgets::chip(format!("Confirmed for {}", self.flavor.label()), strict)
                .on_press(Message::DiscoverStrict(true)),
            widgets::chip("Everything found", !strict).on_press(Message::DiscoverStrict(false)),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// "This zip replaces folders you have": the names, what happens to them,
    /// and the two ways out.
    pub(super) fn confirm_card<'a>(&'a self, p: &'a PendingInstall) -> Element<'a, Message> {
        let n = p.prepared.conflicts.len();
        let what = if n == 1 {
            "a folder".to_string()
        } else {
            format!("{n} folders")
        };
        let body = format!(
            "{} from {} replaces {what} already in AddOns: {}. They go to the backup; Rollback restores them.",
            p.name,
            p.source.provider(),
            ellipsize(&p.prepared.conflicts.join(", "), 300),
        );
        let actions = row![
            widgets::ghost("Cancel", size::META, theme::space::BTN_SM).on_press(Message::CancelInstall),
            widgets::danger("Replace", size::META, theme::space::BTN_SM)
                .on_press_maybe((!self.busy).then_some(Message::ConfirmInstall)),
        ]
        .spacing(10);
        widgets::confirm_banner(format!("Replace {what}?"), body, actions)
    }

    /// "Download 453 MB?": a fresh install whose zip is far larger than most
    /// (a voice-over pack), asked about before a byte of it is fetched.
    /// Nothing is replaced, so the answer is the primary button, not danger.
    pub(super) fn large_card<'a>(&'a self, r: &'a ResolvedInstall) -> Element<'a, Message> {
        let size = human_size(r.release.size.unwrap_or_default());
        let body = format!(
            "{} from {} is a {size} zip, far larger than most addons. Installing it takes about \
             twice that on disk for a moment; the zip is deleted afterwards.",
            r.name,
            r.source.provider(),
        );
        let actions = row![
            widgets::ghost("Cancel", size::META, theme::space::BTN_SM).on_press(Message::CancelDownload),
            widgets::primary("Download", size::META, theme::space::BTN_SM)
                .on_press_maybe((!self.busy).then_some(Message::ConfirmDownload)),
        ]
        .spacing(10);
        widgets::confirm_banner(format!("Download {size}?"), body, actions)
    }

    /// One line on the WoWInterface catalog: loading, ready, or failed with a
    /// retry.
    fn catalog_status(&self) -> Element<'_, Message> {
        match &self.discover.catalog {
            CatalogState::Idle => space::vertical().height(0).into(),
            CatalogState::Loading => {
                self.loader("Downloading the WoWInterface catalog (about 6 MB, once per session)…")
            }
            CatalogState::Ready(n) => text(format!("WoWInterface catalog ready: {n} addons."))
                .size(size::META)
                .style(theme::muted)
                .into(),
            CatalogState::Failed(e) => row![
                text(format!("WoWInterface catalog unavailable: {e}"))
                    .size(size::META)
                    .style(theme::tone(Tone::Update))
                    .width(Fill),
                widgets::ghost("Retry", size::META, [4, 10]).on_press(Message::LoadCatalog),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
        }
    }

    /// Three squares lit in turn, then `label`. Driven by `tick`, which only
    /// advances while [`App::subscription`] sees something loading.
    pub(super) fn loader<'a>(&self, label: &'a str) -> Element<'a, Message> {
        let lit = self.tick % 3;
        let dots = (0..3).fold(row![].spacing(6), |r, i| {
            r.push(
                container(space::horizontal().width(0))
                    .width(Length::Fixed(6.0))
                    .height(Length::Fixed(6.0))
                    .style(theme::loader_dot(i == lit)),
            )
        });
        row![dots, text(label).size(size::META).style(theme::muted)]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
    }

    /// One search result: what it is on the left, one line per site offering
    /// it on the right, each with its own Install button.
    fn found_row<'a>(&'a self, index: usize, f: &'a Found) -> Element<'a, Message> {
        let installed = self.installed.contains(f);
        let primary = f.primary();
        let authors = f.authors();
        let mut name_col = column![text(&primary.name).size(size::ROW).font(theme::UI_MEDIUM)].spacing(2);
        if !authors.is_empty() {
            name_col = name_col.push(
                text(format!("by {}", authors.join(", ")))
                    .size(size::META)
                    .style(theme::muted),
            );
        }
        if let Some(summary) = f.hits.iter().map(|h| h.summary.trim()).find(|s| !s.is_empty()) {
            name_col = name_col.push(text(ellipsize(summary, 180)).size(size::META).style(theme::muted));
        }

        let mut sources = column![].spacing(6).width(Length::Fixed(theme::space::SOURCES_W));
        if installed {
            sources = sources.push(badge("Installed", Tone::Ok));
        } else if f.hits.len() > 1 {
            sources = sources.push(
                text(format!("On {} sites: pick one", f.hits.len()))
                    .size(size::XS)
                    .style(theme::muted),
            );
        }
        let can_install = !installed && !self.busy && !self.asking() && self.active_dir().is_some();
        for h in &f.hits {
            let open = widgets::ghost("Open", size::META, theme::space::BTN_SM).on_press_maybe(
                h.url
                    .starts_with("https://")
                    .then(|| Message::OpenUrl(h.url.clone())),
            );
            // "3.0M downloads · 2025": the year tells a stale upload apart.
            let facts: Vec<String> = h
                .popularity
                .map(|p| p.describe())
                .into_iter()
                .chain(h.updated.map(|y| y.to_string()))
                .collect();
            // Cut at the edge rather than wrapped: the line stays one row
            // when the badges leave the facts little room.
            let facts = container(widgets::one_line(
                text(facts.join(" · ")).size(size::META).style(theme::muted),
            ))
            .width(Fill)
            .clip(true);
            let site_only = !installed && (h.site_only || self.discover.site_only.contains(&h.source));
            let tip_open = self.discover.tip.as_ref() == Some(&h.source);
            let mut line = row![
                text(h.source.provider())
                    .size(size::CONTROL)
                    .width(Length::Fixed(96.0)),
                facts,
                badge(h.compat.label(self.flavor), fit_tone(h.compat)),
                // One "?" a line: it explains the fit, and "Site only" too.
                widgets::help(tip_open, Message::ToggleTip(h.source.clone())),
                open,
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            if site_only {
                // Nothing to download for an app: Open, beside it, is the way.
                line = line.push(badge("Site only", Tone::Manual));
            } else if !installed {
                let running = self.discover.installing.as_ref() == Some(&h.source);
                line = line.push(
                    widgets::primary(
                        if running { "Installing…" } else { "Install" },
                        size::META,
                        theme::space::BTN_SM,
                    )
                    .on_press_maybe(
                        can_install.then(|| Message::InstallFrom(h.source.clone(), h.name.clone())),
                    ),
                );
            }
            sources = sources.push(line);
            if tip_open {
                let fit = h.compat.describe(self.flavor, self.client_interface);
                let body = if site_only {
                    format!("{}\n\n{fit}", site_only_reason(h))
                } else {
                    fit
                };
                sources = sources.push(widgets::tip(body));
            }
        }

        // The picture once it has arrived; the monogram meanwhile, or for good.
        let picture = self.icon(f.icon());
        // Icon and name open the addon's page, when a site has one.
        let has_page = f.hits.iter().any(|h| tocsin_core::page::has_page(&h.source));
        let named = button(
            row![
                icon_slot(monogram(&primary.name), false, false, picture),
                name_col.width(Fill)
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        )
        .padding(Padding::ZERO)
        .width(Fill)
        .style(theme::name_link(false))
        .on_press_maybe(has_page.then_some(Message::OpenFoundPage(self.discover.seq, index)));
        container(row![named, sources].spacing(14).align_y(Alignment::Center))
            .padding(theme::space::ROW_PAD)
            .style(theme::row)
            .into()
    }
}

/// Why a line says "Site only", for its "?": the search answer flagged it
/// (a CurseForge author's choice), or an install found nothing to fetch.
fn site_only_reason(h: &Hit) -> &'static str {
    match h.source {
        _ if h.site_only => {
            "The author lets CurseForge hand the files out only on its website and in its own app, so \
             Tocsin gets no download link. Open the page, download the zip and unzip it into AddOns, or \
             pick another site if one is listed."
        }
        SourceRef::Github { .. } => {
            "This repository has no release to download, only its source code. Tocsin found out when you \
             pressed Install, and remembers it until it closes. Open the page to see how the author \
             expects it to be installed, or pick another site if one is listed."
        }
        _ => {
            "This site gave no file to download for it. Open the page and install it by hand, or pick \
             another site if one is listed."
        }
    }
}

/// The badge tone of what a site says about this game client: green when it
/// lists this build, orange for an older one, violet for a newer one, blue
/// when it does not say. Other games are dropped by the search; red if one
/// ever slips through.
fn fit_tone(c: Compat) -> Tone {
    match c {
        Compat::Exact => Tone::Ok,
        Compat::Older { .. } => Tone::Update,
        Compat::Newer { .. } => Tone::Ahead,
        Compat::Unknown => Tone::Manual,
        Compat::OtherFlavor => Tone::Error,
    }
}

/// A one-line hint with a button to the Settings page; `warn` tints it.
fn hint_row<'a>(message: &'a str, warn: bool, action: &'a str) -> Element<'a, Message> {
    let style: fn(&iced::Theme) -> text::Style = if warn {
        |t| theme::tone(Tone::Update)(t)
    } else {
        theme::muted
    };
    row![
        text(message).size(size::META).style(style).width(Fill),
        widgets::ghost(action, size::META, [4, 10]).on_press(Message::Show(Screen::Settings)),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_tones_follow_the_status_hues() {
        assert_eq!(fit_tone(Compat::Exact), Tone::Ok);
        assert_eq!(fit_tone(Compat::Older { built_for: 120100 }), Tone::Update);
        assert_eq!(fit_tone(Compat::Newer { built_for: 16002 }), Tone::Ahead);
        assert_eq!(fit_tone(Compat::Unknown), Tone::Manual);
    }
}
