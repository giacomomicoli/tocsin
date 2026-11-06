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

### What's new in 0.2.0

- **Search knows which game you play.** *Get addons* now searches for the
  game folder you have selected. CurseForge is asked for that game only, and
  WoWInterface and Tukui results made for other games are dropped. Each site
  line says whether the site lists a file for your game, an older or newer
  one (*Out of date*, *Newer build*), or says nothing (*Unverified*). A **?**
  next to it explains why. You can keep only confirmed results or see
  everything. A Classic client gets only addons for its own expansion. To
  make this work, a CurseForge search now tells CurseForge which version of
  the game you play.
- **Better GitHub updates.** When an addon's GitHub release says which games
  each zip is for, Tocsin picks the zip for yours. The library then shows
  *Out of date* or *Newer build* under the name of an available update.
- **"Site only" instead of an Install that can't work.** Some CurseForge
  authors don't let apps download their files, and some GitHub projects have
  no release to download. Their search results now say *Site only* and
  offer **Open**. The README explains when a GitHub token helps and how to
  make one.
- **Remove a game folder.** Use **Remove folder** at the top of the library,
  the **×** on an installation in the sidebar, or *Settings → Game Folders*.
  Tocsin asks first, then forgets the folder; nothing on disk is touched.
  Removing a folder other than the one on screen no longer empties the
  library.
- **Each game's icon in the sidebar.** Tocsin reads the icon from the
  game's own executable in the folder you added. It doesn't ship any
  Blizzard art, and a folder without a usable icon shows two letters.
- **Versions fit their column.** A long version is cut with `…` and shown
  whole when you hover it. File names around a version are dropped, so
  `AztarecHelper-2.9.0.zip` shows as `2.9.0`.
- **Fixed:** an addon whose name contains "rc", "pre", "dev" and the like
  (`WarcraftLogs`, `Preydator`, `DevTool`) was taken for a pre-release. Its
  update check could miss an update, or call your copy newer than the site's.
