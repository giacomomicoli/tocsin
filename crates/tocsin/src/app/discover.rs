//! The "Get addons" page: search across providers, one row per addon with
//! a line per site offering it, and the "replace these folders?" card.

use iced::widget::{column, container, row, space, text, text_input, Column};
use iced::{Alignment, Element, Fill, Length};

use tocsin_core::search::Found;
use tocsin_core::secrets::Secret;

use super::widgets::{self, badge, icon_slot, monogram};
use super::{App, CatalogState, Message, PendingInstall, Screen};
use crate::theme::{self, size, Tone};

impl App {
    pub(super) fn discover(&self) -> Element<'_, Message> {
        let d = &self.discover;
        let target = match self.active_dir() {
            Some(dir) if self.client.is_empty() => format!("Installs into {}", dir.display()),
            Some(dir) => format!("Installs into {} · {}", self.client, dir.display()),
            None => "Add a game folder in Settings to install addons.".to_string(),
        };
        let bar = row![
            text_input("Search addons by name…", &d.query)
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
        let list = d
            .outcome
            .results
            .iter()
            .fold(Column::new().spacing(theme::space::ROW_GAP), |col, f| {
                col.push(self.found_row(f))
            });
        page.push(widgets::scroll(list)).into()
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
        container(
            row![
                column![
                    text(format!("Replace {what}?"))
                        .size(size::CHILD)
                        .font(theme::DISPLAY)
                        .style(theme::tone(Tone::Update)),
                    text(body).size(size::CONTROL).style(theme::body),
                ]
                .spacing(2)
                .width(Fill),
                widgets::ghost("Cancel", size::META, theme::space::BTN_SM).on_press(Message::CancelInstall),
                widgets::danger("Replace", size::META, theme::space::BTN_SM)
                    .on_press_maybe((!self.busy).then_some(Message::ConfirmInstall)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([12, 16])
        .style(theme::confirm)
        .into()
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
    fn loader<'a>(&self, label: &'a str) -> Element<'a, Message> {
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
    fn found_row<'a>(&'a self, f: &'a Found) -> Element<'a, Message> {
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
        let can_install =
            !installed && !self.busy && self.discover.confirm.is_none() && self.active_dir().is_some();
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
            let mut line = row![
                text(h.source.provider())
                    .size(size::CONTROL)
                    .width(Length::Fixed(96.0)),
                text(facts.join(" · "))
                    .size(size::META)
                    .style(theme::muted)
                    .width(Fill),
                open,
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            if !installed {
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
        }

        // The picture once it has arrived; the monogram meanwhile, or for good.
        let picture = self.icon(f.icon());
        container(
            row![
                icon_slot(monogram(&primary.name), false, false, picture),
                name_col.width(Fill),
                sources
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        )
        .padding(theme::space::ROW_PAD)
        .style(theme::row)
        .into()
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

/// At most `max` characters of `s`, with an ellipsis when cut.
fn ellipsize(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", s[..at].trim_end()),
        None => s.to_string(),
    }
}
