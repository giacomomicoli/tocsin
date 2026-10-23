# Tocsin under the hood

The [README](../README.md) is for using Tocsin. This page is for the curious:
how it guesses where an addon comes from, where it keeps its files, what it
sends over the network, and how to build it.

## How it finds the source

Every addon ships a `.toc` file, and most of them already say where they come
from:

```
## Version: v12.0.376
## X-Curse-Project-ID: 1472162
## X-WoWI-ID: 17019
## X-Website: https://github.com/Author/Addon
```

Tocsin turns those lines into update sources, best first, and skips the ones
you have no key for. When the `.toc` undersells its addon, a few small,
well-known files in the addon's root are consulted too — read-only, size-capped:

| Signal | Effect |
|---|---|
| GitHub link in `X-Website`, `CHANGELOG` or `README` | GitHub, no key needed |
| `X-WoWI-ID` / `X-Tukui-ProjectID` | WoWInterface / Tukui, no key needed |
| `X-Curse-Project-ID` | CurseForge, with your key |
| Any other `X-Website` | Shown as *Check manually* |
| `$Revision: N $` in `Ver.lua` | Appended when `## Version` is just `9.6` |
| `## Interface` vs the game's build | *No .toc for Retail* / *out of date* decided by numbers, not guesses |
| Same GitHub repo, `## Dependencies` / `## X-Part-Of`, or `## OptionalDeps` by the same author | Grouped as a family |

An addon's companions inherit its repository, so *Routine Data* is checked
against the same place as *Routine*. When one repository releases several
addons under different tags (`v12.0.376` for Routine, `dv12.0.13` for Routine
Data), the right one is picked from the release's file names, or from how
close each is to the installed version.

Which game and build a folder belongs to comes from Blizzard's own
`.flavor.info` and `.build.info` files, so Retail, Classic Era, Classic and
WoW Classic Forever are told apart exactly, and sources that publish one file
per flavor (CurseForge, GitHub release assets) are asked for the right one.

What Tocsin installed is remembered by release, so an author who forgets to
bump `## Version` in the package cannot make it look stale again.

Wago is not supported: its API key is a paid Patreon perk. `X-Wago-ID` lines
in a `.toc` are ignored, and an old `type = "wago"` override in `tocsin.toml`
reads as no override.

## Search

| Searched | Key | How |
|---|---|---|
| WoWInterface | none | Its whole catalog (about 6 MB, 8 000+ addons) is downloaded when you open the page (at most once every six hours), then searched locally: names first, most downloaded this month first |
| Tukui | none | ElvUI and Tukui |
| GitHub | none | Repositories named like your query, Lua only, most starred first |
| CurseForge | yours | Most popular first |

Results from a catalog (CurseForge, WoWInterface, Tukui) come before those
only GitHub knows. GitHub search is best effort: anything named like the
query shows up, including ports for private servers, and whether a
repository publishes a release zip is only known when you press *Install*.
Search runs when you press Enter, not while you type: GitHub allows ten
searches a minute without a token.

## Overriding the guess

For the rare addon Tocsin gets wrong, `tocsin.toml` accepts overrides keyed
by the addon's folder name. It lives next to Tocsin's settings:
`%APPDATA%\tocsin\config\tocsin.toml` on Windows,
`~/.config/tocsin/tocsin.toml` on Linux. See `tocsin.example.toml`.

```toml
[addons.SomeAddon]
source = { type = "github", owner = "someone", repo = "some-addon" }
version = "1.2.3"    # when the .toc lies
ignore = true        # leave it out of checks
```

Keys can also come from the environment (`TOCSIN_CURSEFORGE_KEY`,
`TOCSIN_GITHUB_TOKEN`); those win over the stored ones and show as read-only
in Settings.

## Where things go

- Settings from earlier builds, which used another folder on Windows and
  macOS, are moved over once at start-up (by rename, never through a link,
  and only when the new folder does not exist yet).
- Downloads: `%LOCALAPPDATA%\tocsin\cache\downloads\` (Windows),
  `~/.cache/tocsin/downloads/` (Linux). A zip is deleted as soon as it is
  installed (Rollback uses the backup folders, not the zip), and anything
  left there for more than a week is removed at start-up. Safe to delete at
  any time.
- Library icons: `%LOCALAPPDATA%\tocsin\cache\icons\` (Windows),
  `~/.cache/tocsin/icons/` (Linux). A few KB each, fetched again after a
  week and removed at start-up once older than that. Safe to delete.
- Previous versions and the install log: `AddOns\.tocsin-backup\`. Also safe
  to delete; you just lose Rollback and Restore.
- Disabled addons: `AddOns\.tocsin-disabled\`. Move a folder back up a level
  to re-enable it by hand.
- Unpacking happens in `AddOns\.tocsin-staging\`, which is empty between
  operations and cleaned at start-up.

[install-design.md](install-design.md) explains the install mechanics and why.

## Security posture

Tocsin reads your AddOns folder and talks to the public APIs of GitHub,
CurseForge, Tukui and WoWInterface over HTTPS, plus the two image hosts of
addon icons. That's the whole surface, and the code is written to keep it
that way:

- `#![forbid(unsafe_code)]` everywhere; TLS through rustls, no OpenSSL.
- HTTPS enforced before any request leaves the process; short timeouts;
  response bodies capped at 1 MiB while they stream (a compressed or chunked
  answer is counted as it arrives, never buffered first); at most six
  requests in flight. The one exception is WoWInterface's catalog, capped at
  16 MiB (it is about 6 MB), decoded off the async workers, kept in memory
  only, and fetched at most once every six hours.
- Files it reads: `.toc`, `CHANGELOG`/`README`, `Ver.lua` in each addon, and
  Blizzard's `.flavor.info` / `.build.info` — top level only, size-capped,
  never through a symlink or junction (a linked `.toc` or doc file is
  ignored, a linked addon folder is skipped). Outside the game folder: its
  own `tocsin.toml`, and its cached icons (256 KiB each at most, decoded
  under the same limits as a download). Fonts and logos are built into the
  program; no asset file is read at runtime.
- Writes only inside `AddOns` (the addon folders it installs, plus its three
  dot-folders) and its own cache directory. Never `WTF`, never anything else.
  If one of the dot-folders, or an addon folder about to be moved, is a link,
  Tocsin refuses to touch it rather than write wherever it points.
- Settings and cache from earlier builds are moved over once at start-up:
  one rename per folder, only when the new folder does not exist yet, refused
  when the old folder or its parent is a link; nothing is copied or overwritten.
- The install log (`AddOns\.tocsin-backup\installs.toml`) decides what gets
  moved and deleted, so it is not trusted blindly: it is size-capped, and a
  record naming anything but plain folder names and its own backup folder is
  dropped on load.
- Redirects are checked before they are followed: an API request stays on
  the host it was sent to, and a download may only hop between the
  providers' own hosts (GitHub → its asset CDN, say), over HTTPS, three hops
  at most.
- Downloads come only from the providers' own hosts; a zip is inspected
  before extraction (no `..`, absolute paths, symlinks, Windows device names
  such as `CON` or `aux.lua`, names ending in a dot or space, dot-folders at
  the root; folders only at the root) and verified against the provider's
  digest when there is one. Zip-bomb limits count the bytes actually
  unpacked, not the sizes the zip claims. Folders are swapped with renames,
  undone if any step fails.
- Installing something new from search never replaces an existing folder
  silently: when the zip ships folders you already have (a bundled `Ace3`
  or `LibStub`, say) Tocsin names them and asks first.
- A search sends what you typed (trimmed, 64 characters at most) to the
  GitHub and CurseForge search endpoints, and nothing else about you or your
  addons; WoWInterface and Tukui are searched locally in their catalogs.
  Installing a result goes through the same download checks as an update.
- Addon icons come only from `media.forgecdn.net` (CurseForge logos) and
  `cdn-wow.mmoui.com` (WoWInterface thumbnails), over HTTPS, checked on
  every redirect hop; nothing is sent but the request for the picture. A
  file is read up to 256 KiB, four at a time, only for the addons and
  results on screen. Its format is taken from its first bytes, PNG or JPEG
  only; the decoder refuses pictures over 512 pixels a side or 4 MiB of
  memory, and runs off the interface thread. Decoded icons are kept in
  memory (300 at most, about 5 MiB); one that fails shows the two letters
  instead.
- Library icons are also cached on disk (see *Where things go*) as Tocsin's
  own 64-pixel PNGs, named after a hash of the address; a cached file is
  read back under the same size cap and decoder limits, never through a
  link. Search icons are never written to disk.
- To find the logos of your installed addons, Tocsin sends CurseForge (with
  your key) the CurseForge ids of every installed addon that has one, 20 to
  a request, once per session; the update check alone only asks about the
  addons CurseForge actually serves. No key, no request. WoWInterface
  thumbnails come from its catalog only when you already opened *Get
  addons*: the 6 MB catalog is never downloaded for icons.
- Only `https://` links are ever handed to your browser.
- Keys live in the OS credential store. They are read once per session, are
  never shown back in Settings (only whether one is set), and debug output
  prints "set" in their place. There is no key in this repository and no key
  baked into the binary.
- `cargo deny` (advisories, licenses, bans) runs in CI on every push;
  dependencies are pinned by `Cargo.lock` and bumped by Dependabot.
- Releases are built only by the GitHub release workflow, never on a
  developer's machine. Each release zip and the exe inside it carry a GitHub
  build provenance attestation that ties them to the commit and workflow run
  that produced them; `gh attestation verify <file> --repo giacomomicoli/tocsin`
  checks it.

Found something? See [SECURITY.md](../SECURITY.md).

## Building from source

Requires a stable Rust toolchain (https://rustup.rs). On Linux you also need
Vulkan drivers, `libxkbcommon`, `xdg-desktop-portal` (file dialog) and a
Secret Service provider such as GNOME Keyring or KWallet (API keys).

```sh
cargo run --release -p tocsin                                  # the app
cargo run -p tocsin-core --example check -- <path to AddOns>   # headless check
cargo test --workspace
```

Release builds use LTO and strip symbols; on Windows the console is hidden.
[fixtures/README.md](../fixtures/README.md) explains how to try Tocsin on a
fake AddOns folder.

Before opening a pull request, run `cargo fmt --all`,
`cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace`; CI runs the same checks plus `cargo deny`.
