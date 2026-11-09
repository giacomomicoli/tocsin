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
  [Getting started on Linux](https://github.com/giacomomicoli/tocsin#on-linux)
  for the game folder inside a Wine prefix and the menu entry.

This release is not signed. On Windows, a "Windows protected your PC" window
may show the first time (click *More info*, then *Run anyway*), and PCs with
Smart App Control turned on won't run it. On Linux nothing warns you, and
nothing vouches for the file either: the checksum and the build attestation
are how you check it. See
[Why the releases aren't signed](https://github.com/giacomomicoli/tocsin#why-the-releases-arent-signed).

Prefer not to run a downloaded program? The same build takes three commands
from the source, on either system: see
[Building it yourself](https://github.com/giacomomicoli/tocsin#building-it-yourself).

### What's new in 0.4.0

Tocsin now runs on Linux. On Windows nothing changes: same program, same
zip.

- **A Linux download.** Next to the Windows zip there is now a `.tar.gz`
  for 64-bit Linux, built by GitHub the same way, with the same checksum
  and build attestation. Unpack it and run `./tocsin`: it needs nothing
  installed beyond what a desktop already has (glibc 2.35 or newer, so
  Ubuntu 22.04, Debian 12, Fedora 36 and later). World of Warcraft under
  Wine (Lutris, Bottles, Steam) is managed like on Windows: pick the game
  folder inside the Wine prefix. Hidden folders such as `~/.wine` show up
  in the file window with Ctrl+H.
- **Its name and icon on your desktop.** The tarball carries a menu entry
  and the icon, with three lines in the README to put them in place. On
  Wayland that is also what gives the window its icon in the taskbar.
- **It says so when your desktop lacks something.** On a Linux system with
  no file chooser, *Add game folder* now tells you what to install instead
  of doing nothing. With no keyring running, saving an API key tells you
  it can't be kept and which environment variable to use instead.
- **One checksum file for every download.** `SHA256SUMS.txt` now lists both
  the zip and the tarball, and on Linux `sha256sum -c` checks it in one go.
  A release is published only when both builds succeed.
- **The README covers all three ways to get Tocsin**: the Windows zip, the
  Linux tarball, or building it from the source on either system.
  ([#38](https://github.com/giacomomicoli/tocsin/pull/38))
