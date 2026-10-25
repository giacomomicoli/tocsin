Tocsin tells you when your World of Warcraft addons have a newer version out,
and updates them for you when it can, by asking the site each one came from:
CurseForge, WoWInterface, GitHub or Tukui. One small Windows program, no
account, no telemetry, nothing running in the background.

**Download** the `.zip` below, unzip it anywhere and run `tocsin.exe`. To
check that the file is the one published here, see
[Checking the download](https://github.com/giacomomicoli/tocsin#checking-the-download-optional).

This release is not signed: Windows may show a "Windows protected your PC"
window the first time (click *More info*, then *Run anyway*), and PCs with
Smart App Control turned on won't run it. See
[Why the releases aren't signed](https://github.com/giacomomicoli/tocsin#why-the-releases-arent-signed).

### What's new in 0.1.1

- **Only real game folders are accepted.** A folder that isn't part of a
  World of Warcraft install (another game's `AddOns`, a backup copy) is
  refused with a clear message, instead of being added and filled with WoW
  addons. Folders you already added keep working.
- **Every version of the game is found.** Picking the main World of Warcraft
  folder now finds PTR, Anniversary and Forever installs too, and a game
  that has never had addons gets told how to start.
- **The right game, the right files.** Tocsin reads the game's build to tell
  versions apart (the Classic beta now shows as Forever), and picks each
  addon's `.toc` the way the game does: TBC Anniversary, Mists, Forever and
  Retail each load their own file first.
- If a folder holds no addons at all, Tocsin says so instead of showing an
  empty list.
