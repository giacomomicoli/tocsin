//! The library page: page head, filter chips, the addon list grouped by
//! family, and the ghost rows for missing dependencies and removed families.

use iced::widget::{column, container, row, text, text_input, Column};
use iced::{Alignment, Element, Fill, Length, Padding, Theme};

use tocsin_core::{InstalledAddon, SourceRef, UpdateStatus};

use super::widgets::{self, badge, icon_slot, monogram};
use super::{App, Counts, Filter, Message};
use crate::theme::{self, size, space, Tone};

impl App {
    /// Head ("Installed Addons", client and path, Rescan / Check updates),
    /// filter chips, then the list.
    pub(super) fn library_page(&self) -> Element<'_, Message> {
        let subtitle = match self.active_dir() {
            Some(dir) if self.client.is_empty() => dir.display().to_string(),
            Some(dir) => format!("{} · {}", self.client, dir.display()),
            None => "No game folder yet".to_string(),
        };
        let actions = row![
            text(format!("{} addons", self.addons.len()))
                .size(size::META)
                .style(theme::muted),
            row![
                widgets::ghost("Rescan", size::CONTROL, space::BTN_MD)
                    .on_press_maybe((!self.busy).then_some(Message::Scan)),
                widgets::primary(
                    if self.busy { "Working…" } else { "Check updates" },
                    size::CONTROL,
                    space::BTN_MD
                )
                .on_press_maybe((!self.busy && !self.addons.is_empty()).then_some(Message::CheckUpdates)),
            ]
            .spacing(8),
        ]
        .spacing(14)
        .align_y(Alignment::Center);
        let mut page = column![
            widgets::page_head("Installed Addons", subtitle, Some(actions.into())),
            self.toolbar(),
        ]
        .spacing(space::PAGE_GAP);
        if let Some(key) = &self.confirm_delete {
            page = page.push(self.delete_banner(key));
        }
        page.push(self.list()).into()
    }

    /// "Delete X?": nothing comes back after this, so the user is asked.
    fn delete_banner<'a>(&'a self, key: &'a str) -> Element<'a, Message> {
        let name = self
            .addons
            .iter()
            .find(|a| a.folder == key)
            .map_or(key, |a| a.display_name.as_str());
        let body = format!(
            "{name} and its kept copy are deleted from AddOns. This cannot be undone; \
             its saved settings stay, so reinstalling it picks them back up."
        );
        let actions = row![
            widgets::ghost("Cancel", size::META, space::BTN_SM).on_press(Message::CancelDelete),
            widgets::danger("Delete", size::META, space::BTN_SM)
                .on_press_maybe((!self.busy).then_some(Message::ConfirmDelete)),
        ]
        .spacing(10);
        widgets::confirm_banner(format!("Delete {name}?"), body, actions)
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let Counts {
            outdated, attention, ..
        } = self.counts;

        let chip = |label: String, f: Filter| {
            widgets::chip(label, self.filter == f).on_press(Message::FilterChanged(f))
        };

        row![
            chip("All".into(), Filter::All),
            chip(format!("Updates {outdated}"), Filter::Outdated),
            chip(format!("Needs attention {attention}"), Filter::Attention),
            iced::widget::space::horizontal(),
            text_input("Search…", &self.search)
                .on_input(Message::SearchChanged)
                .padding(8)
                .size(size::CONTROL)
                .width(Length::Fixed(space::SEARCH_W))
                .style(theme::input),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    fn passes(&self, idx: usize, a: &InstalledAddon, needle: &str) -> bool {
        let status = self.statuses.get(&a.folder);
        let pass_filter = match self.filter {
            Filter::All => true,
            Filter::Outdated => matches!(status, Some(UpdateStatus::Outdated(_))),
            Filter::Attention => {
                self.missing.contains_key(&idx)
                    || a.issues.iter().any(|i| i.is_blocking())
                    || status.is_some_and(UpdateStatus::needs_attention)
            }
        };
        pass_filter
            && (needle.is_empty()
                || a.display_name.to_lowercase().contains(needle)
                || a.folder.to_lowercase().contains(needle))
    }

    fn list(&self) -> Element<'_, Message> {
        if self.addons.is_empty() {
            let hint = if self.busy {
                "Scanning…"
            } else {
                "No addons found. Check the game folder in Settings."
            };
            return container(text(hint).size(size::CONTROL).style(theme::muted))
                .center(Fill)
                .into();
        }
        let needle = self.search.trim().to_lowercase();
        let mut col = Column::new().spacing(space::ROW_GAP);
        for fam in &self.families {
            let head = &self.addons[fam.head];
            let members: Vec<usize> = fam
                .members
                .iter()
                .copied()
                .filter(|&i| self.passes(i, &self.addons[i], &needle))
                .collect();
            // A family shows when its head passes or any member does.
            if !self.passes(fam.head, head, &needle) && members.is_empty() {
                continue;
            }
            col = col.push(self.addon_row(head, false));
            for &i in &members {
                col = col.push(self.addon_row(&self.addons[i], true));
            }
            // Ghost rows for hard dependencies that are not installed.
            for idx in std::iter::once(fam.head).chain(members.iter().copied()) {
                if let Some(deps) = self.missing.get(&idx) {
                    for d in deps {
                        let head_folder = &self.addons[fam.head].folder;
                        // Installable when the head resolves through a provider
                        // that can serve a file by name (anything but Manual).
                        let via_head = self.addons[fam.head]
                            .source()
                            .is_some_and(|s| !matches!(s, SourceRef::Manual { .. }));
                        col = col.push(self.missing_row(
                            d,
                            &self.addons[idx].display_name,
                            via_head.then(|| head_folder.clone()),
                        ));
                    }
                }
            }
        }
        for (key, version) in &self.removed {
            col = col.push(self.removed_row(key, version));
        }
        widgets::scroll(col).into()
    }

    fn removed_row<'a>(&'a self, key: &'a str, version: &'a str) -> Element<'a, Message> {
        let name_col = column![
            text(key).size(size::CHILD).font(theme::UI_MEDIUM),
            text(format!("removed · {version} kept until the next change"))
                .size(size::META)
                .style(theme::faint),
        ]
        .spacing(2);
        container(
            row![
                icon_slot(monogram(key), true, true, None),
                name_col.width(Fill),
                row![
                    widgets::ghost("Restore", size::META, space::BTN_SM)
                        .on_press_maybe((!self.busy).then(|| Message::Rollback(key.to_string()))),
                    widgets::danger("Delete", size::META, space::BTN_SM)
                        .on_press_maybe((!self.busy).then(|| Message::Delete(key.to_string()))),
                ]
                .spacing(8),
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        )
        .padding(space::ROW_CHILD_PAD)
        .style(theme::row_disabled)
        .into()
    }

    fn missing_row<'a>(
        &'a self,
        folder: &'a str,
        needed_by: &'a str,
        install_via: Option<String>,
    ) -> Element<'a, Message> {
        let name_col = column![
            text(folder).size(size::CHILD).font(theme::UI_MEDIUM),
            text(format!("required by {needed_by} · not installed"))
                .size(size::META)
                .style(theme::tone(Tone::Error)),
        ]
        .spacing(2);
        let inner = container(
            row![
                text("└").size(size::CHILD).style(theme::faint),
                icon_slot(monogram(folder), true, true, None),
                name_col.width(Fill),
                container(badge("Missing", Tone::Error))
                    .width(Length::Fixed(space::BADGE_COL_W))
                    .align_x(Alignment::End),
                widgets::primary("Install", size::META, space::BTN_SM).on_press_maybe(
                    (!self.busy)
                        .then_some(install_via)
                        .flatten()
                        .map(|head| Message::InstallMissing(folder.to_string(), head)),
                ),
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        )
        .padding(space::ROW_CHILD_PAD)
        .style(theme::row_missing);
        container(inner)
            .padding(Padding::ZERO.left(space::CHILD_INDENT))
            .into()
    }

    fn addon_row<'a>(&'a self, a: &'a InstalledAddon, child: bool) -> Element<'a, Message> {
        let status = self.statuses.get(&a.folder);

        let installed = a
            .installed
            .as_ref()
            .map(|v| v.as_str().to_string())
            .unwrap_or_else(|| "—".into());

        let latest = match status {
            Some(
                UpdateStatus::UpToDate(r)
                | UpdateStatus::Outdated(r)
                | UpdateStatus::Ahead(r)
                | UpdateStatus::Unknown(r),
            ) => {
                // A fallback provider answered: say which, since it may differ
                // in version string and may offer no direct download.
                let first = a.source().map(SourceRef::provider).unwrap_or("");
                if r.provider.is_empty() || r.provider == first {
                    r.version.as_str().to_string()
                } else {
                    format!("{} · {}", r.version.as_str(), r.provider)
                }
            }
            _ => "—".into(),
        };

        let (badge_text, badge_tone) = match status {
            _ if a.disabled => ("Disabled", Tone::Muted),
            None => (if self.busy { "Checking…" } else { "Not checked" }, Tone::Muted),
            Some(s @ UpdateStatus::Outdated(_)) => (s.label(), Tone::Update),
            Some(s @ UpdateStatus::UpToDate(_)) => (s.label(), Tone::Ok),
            Some(s @ UpdateStatus::Ahead(_)) => (s.label(), Tone::Ahead),
            Some(s @ (UpdateStatus::Unknown(_) | UpdateStatus::Manual(_))) => (s.label(), Tone::Manual),
            Some(s @ UpdateStatus::Error(_)) => (s.label(), Tone::Error),
            Some(s @ (UpdateStatus::NoSource | UpdateStatus::Ignored)) => (s.label(), Tone::Muted),
        };

        let source_label = a.source().map(SourceRef::provider).unwrap_or("—");
        let link = match (status, a.source()) {
            (
                Some(
                    UpdateStatus::Outdated(r)
                    | UpdateStatus::UpToDate(r)
                    | UpdateStatus::Ahead(r)
                    | UpdateStatus::Unknown(r),
                ),
                _,
            ) => Some(r.url.clone()),
            (_, Some(src)) => Some(src.web_url()),
            _ => None,
        }
        .filter(|u| u.starts_with("https://"));

        let open_btn =
            widgets::ghost("Open", size::META, space::BTN_SM).on_press_maybe(link.map(Message::OpenUrl));

        // Update: only when outdated with a direct download. A member whose
        // release is the same zip as its head's shows no button of its own.
        let head_download = self
            .head_of
            .get(&a.folder)
            .and_then(|h| match self.statuses.get(h) {
                Some(UpdateStatus::Outdated(r)) => r.download.as_deref(),
                _ => None,
            });
        let updatable = match status {
            Some(UpdateStatus::Outdated(r)) => {
                !a.disabled && r.download.is_some() && r.download.as_deref() != head_download
            }
            _ => false,
        };
        let is_installing = self.installing.as_deref() == Some(&a.folder);
        let update_btn = widgets::primary(
            if is_installing { "Installing…" } else { "Update" },
            size::META,
            space::BTN_SM,
        )
        .on_press_maybe((updatable && !self.busy).then(|| Message::Update(a.folder.clone())));
        let rollback_btn = widgets::ghost("Rollback", size::META, space::BTN_SM).on_press_maybe(
            (!self.busy && self.rollbackable.contains(&a.folder))
                .then(|| Message::Rollback(a.folder.clone())),
        );

        let detail: Option<String> = match status {
            Some(UpdateStatus::Error(e)) => Some(e.clone()),
            Some(UpdateStatus::Manual(url)) => Some(format!("Check {url}")),
            Some(UpdateStatus::Outdated(r)) if r.download.is_none() && !a.disabled => {
                Some(format!("{} offers no direct download — use Open", r.provider))
            }
            _ => None,
        };

        let meta_style: fn(&Theme) -> text::Style = if a.disabled { theme::faint } else { theme::muted };
        let mut name_col = column![
            text(&a.display_name)
                .size(if child { size::CHILD } else { size::ROW })
                .font(theme::UI_MEDIUM),
            text(format!("{} · {}", a.folder, source_label))
                .size(size::META)
                .style(meta_style),
        ]
        .spacing(2);
        if let Some(d) = detail {
            name_col = name_col.push(text(d).size(size::META).style(theme::tone(Tone::Update)));
        }
        for issue in &a.issues {
            let style: fn(&Theme) -> text::Style = if issue.is_blocking() {
                |t| theme::tone(Tone::Error)(t)
            } else {
                theme::muted
            };
            name_col = name_col.push(text(issue.describe()).size(size::META).style(style));
        }

        let versions = column![
            text(installed).size(size::CONTROL).font(theme::MONO),
            text(latest)
                .size(size::META)
                .font(theme::MONO)
                .style(theme::muted),
        ]
        .spacing(2)
        .align_x(Alignment::End)
        .width(Length::Fixed(space::VERSIONS_W));

        let mut actions = row![open_btn].spacing(8);
        if updatable || is_installing {
            actions = actions.push(update_btn);
        }
        if self.rollbackable.contains(&a.folder) {
            actions = actions.push(rollback_btn);
        }
        if !child {
            let toggle = if a.disabled {
                widgets::ghost("Enable", size::META, space::BTN_SM)
                    .on_press_maybe((!self.busy).then(|| Message::Enable(a.folder.clone())))
            } else {
                widgets::ghost("Disable", size::META, space::BTN_SM)
                    .on_press_maybe((!self.busy).then(|| Message::Disable(a.folder.clone())))
            };
            let remove = widgets::danger("Remove", size::META, space::BTN_SM)
                .on_press_maybe((!self.busy).then(|| Message::Remove(a.folder.clone())));
            actions = actions.push(toggle).push(remove);
            // Out of the game's sight already: offer to be rid of it for good.
            if a.disabled {
                actions = actions.push(
                    widgets::danger("Delete", size::META, space::BTN_SM)
                        .on_press_maybe((!self.busy).then(|| Message::Delete(a.folder.clone()))),
                );
            }
        }

        let mut r = row![].spacing(14).align_y(Alignment::Center);
        if child {
            r = r.push(text("└").size(size::CHILD).style(theme::faint));
        }
        let r = r
            .push(icon_slot(
                monogram(&a.display_name),
                child,
                a.disabled,
                self.icon(self.library_icons.get(&a.folder).map(String::as_str)),
            ))
            .push(name_col.width(Fill))
            .push(versions)
            .push(
                container(badge(badge_text, badge_tone))
                    .width(Length::Fixed(space::BADGE_COL_W))
                    .align_x(Alignment::End),
            )
            .push(actions);

        let inner = container(r)
            .padding(if child {
                space::ROW_CHILD_PAD
            } else {
                space::ROW_PAD
            })
            .style(if a.disabled {
                theme::row_disabled
            } else if child {
                theme::row_child
            } else {
                theme::row
            });
        if child {
            container(inner)
                .padding(Padding::ZERO.left(space::CHILD_INDENT))
                .into()
        } else {
            inner.into()
        }
    }
}
