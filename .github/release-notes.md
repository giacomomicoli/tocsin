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

### What's new in 0.3.0

- **A page for every addon.** Click an addon's name, in the library or in
  the search results, to read what its site says about it: the summary, the
  screenshots and the full description (CurseForge's page, the GitHub
  README, the WoWInterface or Tukui description). When several sites have a
  page, one button per site switches between them; **Back** returns to the
  list where you left it, and a search result's **Install** is right there.
  Pictures come only from the addon sites and from `i.imgur.com`; any other
  is shown as a link. Opening a page tells that site which addon you looked
  at. ([#17](https://github.com/giacomomicoli/tocsin/issues/17))
- **The library says what each addon is.** A short line under the name, when
  its site sends one with the update check (CurseForge, WoWInterface,
  Tukui). The window now opens a little larger, 1280×800, to give the new
  page room.
- **Large addons install.** Voice-over packs of 300–500 MB (the *Spoken*
  addons) were refused as "larger than 256 MiB". Tocsin now takes them when
  the site says how big the file is, asks first on a new install
  ("Download 453 MB?"), shows the download's progress, picks up a dropped
  connection where it stopped, and checks the free space on your disk
  first. ([#26](https://github.com/giacomomicoli/tocsin/issues/26))
- **Addons unzipped one folder too deep.** Windows' *Extract all* puts
  Details' folders inside a folder named after the zip, where the game
  loads none of them. The library now shows such a folder as **Not
  loaded** with **Move up**, which puts the addons where the game looks
  (asking first if that replaces anything, and keeping the old copies for
  *Rollback*). Installing a zip made that way works too.
  ([#27](https://github.com/giacomomicoli/tocsin/issues/27))
- **Fixed: hand-installed CurseForge addons showed an error.** Auctionator
  names only its CurseForge page, not its id: Tocsin now checks it on
  CurseForge (with your key) instead of a GitHub repository that has gone
  private. ([#25](https://github.com/giacomomicoli/tocsin/issues/25))
