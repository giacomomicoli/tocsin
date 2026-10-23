# Security Policy

Tocsin runs on your machine, reads your addon folder and talks to a handful of
public APIs over HTTPS, plus two image hosts for addon icons
(`media.forgecdn.net`, `cdn-wow.mmoui.com`). If you find a way to make it do
anything else, please report it privately rather than opening a public issue.

Areas where a report is most valuable: zip inspection and extraction
(`archive.rs`, `install.rs`), what the install log may make Tocsin move or
delete, redirect and host checks (`http.rs`, `download.rs`, `icons.rs`), and
decoding of downloaded pictures or cached icons (`icons.rs`). The
"Security posture" section of `docs/technical.md` describes what each of these is meant to refuse.

**Contact:** dev@fakejack.mozmail.com (or open a GitHub security advisory on this
repository).

Please include the version (the release number or the commit hash), your OS and
steps to reproduce. You will get an acknowledgement within a few days.

Supported: the latest release on the `main` branch.
