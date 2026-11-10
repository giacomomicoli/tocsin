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

### What's new in 0.4.1

A maintenance release: nothing changes in what you see or do. If 0.4.0
works for you, there is no hurry to replace it.

- **Linux: the check behind *Add game folder* moved to a newer library.**
  When the folder window gives nothing back, Tocsin asks the desktop
  whether it has a file chooser at all, so it can tell you what to install.
  That question now goes through the current version of the D-Bus library
  it uses (zbus 6), which keeps the Linux build in step with its
  dependencies. Same question, same answer, same message. The Linux
  download is about 350 KB larger until another library Tocsin uses makes
  the same move.
  ([#42](https://github.com/giacomomicoli/tocsin/pull/42))
- **Windows: no change.** The zip is rebuilt from the same code as 0.4.0;
  only the version number differs.

New in 0.4.0, for those coming from 0.3: Tocsin runs on Linux, with a
`.tar.gz` next to the Windows zip. See the
[0.4.0 notes](https://github.com/giacomomicoli/tocsin/releases/tag/v0.4.0).
