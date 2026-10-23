//! The settings page: game folders, API keys and the theme, each in a
//! gilded frame.

use iced::widget::{column, container, row, space, text, text_input};
use iced::{Alignment, Element, Fill, Length, Theme};

use tocsin_core::detect;
use tocsin_core::secrets::{KeyStatus, Secret};

use super::widgets::{self, frame};
use super::{App, Message, Typed};
use crate::theme::{self, size, Tone};

impl App {
    pub(super) fn settings(&self) -> Element<'_, Message> {
        let config = self
            .manifest_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let body = column![
            frame(self.folders_section()),
            frame(self.keys_section()),
            frame(self.theme_section()),
            row![
                space::horizontal(),
                widgets::primary(if self.busy { "Saving…" } else { "Save" }, size::CONTROL, [9, 18])
                    .on_press_maybe((!self.busy && self.form.dirty()).then_some(Message::SaveSettings)),
            ],
        ]
        .spacing(16);
        column![
            widgets::page_head("Settings", config, None),
            widgets::scroll(body),
        ]
        .spacing(theme::space::PAGE_GAP)
        .into()
    }

    fn folders_section(&self) -> Element<'_, Message> {
        let folders = &self.manifest.settings.game_folders;
        let mut col = column![
            section_title("Game Folders"),
            description(
                "Pick your World of Warcraft folder (e.g. _retail_) or its Interface folder, like in any addon \
                 manager. Add one per game version you play."
            ),
        ]
        .spacing(10);
        if folders.is_empty() {
            col = col.push(
                text("No game folder added yet.")
                    .size(size::CONTROL)
                    .style(theme::muted),
            );
        }
        for (i, f) in folders.iter().enumerate() {
            let tag = container(
                text(detect::label_of(f).to_uppercase())
                    .size(size::XS)
                    .font(theme::DISPLAY),
            )
            .padding(theme::space::BADGE)
            .style(theme::tag_gold);
            col = col.push(
                container(
                    row![
                        container(tag).width(Length::Fixed(96.0)),
                        text(f.display().to_string())
                            .size(size::CONTROL)
                            .font(theme::MONO)
                            .width(Fill),
                        widgets::ghost("Remove", size::META, [4, 10])
                            .on_press_maybe((!self.busy).then_some(Message::RemoveGameFolder(i))),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding([8, 12])
                .style(theme::row),
            );
        }
        col.push(row![
            space::horizontal(),
            widgets::primary("Add game folder…", size::CONTROL, [9, 14]).on_press(Message::AddGameFolder),
        ])
        .into()
    }

    fn keys_section(&self) -> Element<'_, Message> {
        let mut col = column![
            section_title("API Keys"),
            description(
                "GitHub, WoWInterface and Tukui need nothing. CurseForge only answers with a personal \
                 key: free, two minutes to get, stored in your OS credential manager, never in a file."
            ),
        ]
        .spacing(12);

        for s in Secret::ALL {
            let status = self.key_status.get(&s).copied();
            let typed = self.form.input.get(&s).map(String::as_str).unwrap_or("");
            let (label, placeholder) = match status {
                None => ("reading…".to_string(), ""),
                Some(KeyStatus::Env) => (format!("set by {}", s.env_var()), "set by the environment"),
                Some(KeyStatus::Stored) => ("saved".to_string(), "type a new key to replace it"),
                Some(KeyStatus::Config) => ("set in tocsin.toml".to_string(), "type a key to save it"),
                Some(KeyStatus::NotSet) => ("not set".to_string(), "paste your key"),
            };
            // GitHub answered 401 to this token earlier in the session: say so
            // where it can be fixed. Requests already go out without it.
            let refused = s == Secret::Github && self.github_token_refused();
            let (label, label_style): (String, fn(&Theme) -> text::Style) = if refused {
                (
                    "refused by GitHub: replace or remove it (working without it for now)".to_string(),
                    |t| theme::tone(Tone::Update)(t),
                )
            } else {
                (label, theme::muted)
            };
            let mut input = text_input(placeholder, typed)
                .padding(10)
                .size(size::CONTROL)
                .font(theme::MONO)
                .secure(true)
                .style(theme::input);
            if status.is_some_and(|st| st != KeyStatus::Env) {
                input = input.on_input(move |v| Message::KeyChanged(s, Typed(v)));
            }
            let mut title = row![
                text(s.label()).size(size::CONTROL).font(theme::UI_MEDIUM),
                text(label).size(size::XS).style(label_style),
                space::horizontal(),
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            if status == Some(KeyStatus::Stored) {
                title = title.push(
                    widgets::danger("Remove", size::META, [4, 10])
                        .on_press_maybe((!self.busy).then_some(Message::RemoveKey(s))),
                );
            }
            title = title.push(
                widgets::ghost("Get a key", size::META, [4, 10])
                    .on_press(Message::OpenUrl(s.help_url().to_string())),
            );
            col = col.push(column![title, input].spacing(6));
        }
        col.into()
    }

    /// Tocsin Void (dark) or Tocsin Sunwell (light). Not saved: the app
    /// opens dark.
    fn theme_section(&self) -> Element<'_, Message> {
        column![
            section_title("Theme"),
            row![
                widgets::chip("Void", self.dark).on_press(Message::SetDark(true)),
                widgets::chip("Sunwell", !self.dark).on_press(Message::SetDark(false)),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .into()
    }
}

fn section_title(title: &str) -> Element<'_, Message> {
    text(title)
        .size(size::SECTION)
        .font(theme::DISPLAY)
        .style(theme::title)
        .into()
}

fn description(body: &str) -> Element<'_, Message> {
    text(body).size(size::META).style(theme::muted).into()
}
