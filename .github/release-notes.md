Tocsin tells you when your World of Warcraft addons have a newer version out,
and updates them for you when it can, by asking the site each one came from:
CurseForge, WoWInterface, GitHub or Tukui. One small program for Windows and
Linux, no account, no telemetry, nothing running in the background.

**Download** the file for your system below. To check that it is the one
published here, see
[Checking the download](https://github.com/giacomomicoli/tocsin#checking-the-download-optional).

- **Windows**: the `.zip`. Unzip it anywhere and run `tocsin.exe`.
- **Linux** (x86-64, glibc 2.35 or newer: Ubuntu 22.04, Debian 12, Fedora 36
  and later): the `.tar.gz`. Unpack it anywhere and run `./tocsin`. See
  [Getting started on Linux](https://github.com/giacomomicoli/tocsin#getting-started-on-linux)
  for the game folder inside a Wine prefix and the menu entry.

This release is not signed. On Windows, a "Windows protected your PC" window
may show the first time (click *More info*, then *Run anyway*), and PCs with
Smart App Control turned on won't run it. On Linux nothing warns you, and
nothing vouches for the file either: the checksum and the build attestation
are how you check it. See
[Why the releases aren't signed](https://github.com/giacomomicoli/tocsin#why-the-releases-arent-signed).

### What's new in 0.3.1

Nothing new to learn: this release makes Tocsin show what it is doing, so
a wait is never a blank one.

- **Update checks show each answer as it arrives.** Rows used to say
  *Checking…* until the whole check was over, then flip all at once. Now
  each badge changes the moment its site answers, and the status line
  counts along: *Checking updates · 12 of 55…*. Pressing **Check updates**
  sets the rows back to *Checking…* right away, so the click is visible.
- **Loaders move.** The three diamonds now step while Tocsin scans a
  folder, checks for updates, waits for your API keys or installs an addon,
  and on an addon's page while it and its pictures load. Before, they stood
  still outside search and downloads.
- **Addon pages load in reading order, without jumping.** The large
  screenshot comes first, then the thumbnails, then the pictures of the
  description, three at a time instead of two. A picture still on its way
  keeps its place, at the size the page declares for it when it does, so
  the text under it no longer moves when it appears.
- **A little quicker to the first frame.** The system fonts are read while
  the window and the graphics card are set up, instead of after. The gain
  is small on a warm start and larger on a cold one. Nothing in the app can
  shorten what happens before it draws (Windows checking a new download, a
  cold disk); see the note above about unsigned releases.
  ([#34](https://github.com/giacomomicoli/tocsin/pull/34))
