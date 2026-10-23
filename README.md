# Tocsin

Tocsin tells you when your World of Warcraft addons have a newer version out,
and updates them for you when it can. It doesn't matter where you got them:
CurseForge, WoWInterface, GitHub, Tukui, or a zip a guildmate sent you.

It's one small Windows program. No account, no login, nothing left running in
the background. You open it, it looks at your AddOns folder, and it tells you
what's out of date.

> *tocsin*: an alarm bell. Also, it reads `.toc` files. Sorry.

![Tocsin's library: installed addons with their versions, update badges and actions](docs/screenshot.png)

## Why I made it

Most addon managers want to be *the* way you install addons: a store, a
login, a built-in browser, an icon in the tray. If you installed something by
hand, or from a site they don't cover, they lose track of it.

Tocsin doesn't care how an addon got onto your disk. It looks at what's there
and asks the place it came from whether there's something newer.

## How it works

Every addon comes with a small text file the game reads at login (the `.toc`).
Most of them also say where the addon is published: a CurseForge project, a
WoWInterface page, a GitHub link. Tocsin reads that, asks the site for the
latest version, and compares it with the one you have.

If the addon doesn't say, Tocsin looks for a link in its changelog or readme.
If there's simply no one to ask (addons sold on the author's own site, like
Zygor), it tells you to check by hand and gives you the link.

Addons that come in several folders, like *Deadly Boss Mods* and its modules,
are shown together as one entry so the list stays short.

Tocsin only ever touches your addon folders. Your settings, keybinds and
profiles (the `WTF` folder) are never modified.

## Getting started

1. Download the `.zip` from the [latest release](../../releases/latest) and
   unzip it wherever you like. There's nothing to install: `tocsin.exe` *is*
   the app, and deleting the folder removes it.
2. Run `tocsin.exe`. If Windows shows a blue "Windows protected your PC" window, click
   *More info*, then *Run anyway*. That's Windows being careful with a
   program from a small project it hasn't seen many times yet.
3. The first time, Tocsin asks for your game folder. Pick `_retail_` (or
   `_classic_era_`, `_classic_`) inside your World of Warcraft folder.

From then on you'll see every addon you have, the version you're on, and the
newest one available.

### Checking the download (optional)

Every release is built by GitHub straight from the code in this repository,
never on someone's PC. If you want to be sure the zip you have is the one
that was published, open PowerShell in the folder where you saved it and run:

```powershell
Get-FileHash tocsin-*-windows-x64.zip
```

The long code it prints must match the one in `SHA256SUMS.txt` on the same
release page. If it doesn't, don't run it, and open an issue.

If you use the [GitHub CLI](https://cli.github.com/), you can go one step
further and check that the file was built from this repository:

```powershell
gh attestation verify tocsin-v0.1.0-windows-x64.zip --repo giacomomicoli/tocsin
```

## Using it

### Reading the list

Each addon gets a badge:

| Badge | What it means |
|---|---|
| **Update available** | There's a newer version. |
| **Up to date** | You have the latest. |
| **Ahead of source** | You have something newer than the site shows (a test version, or the site is behind). |
| **Check manually** | Tocsin can't ask anyone about this one. *Open* takes you to its page. |
| **Missing** | Another addon needs this one, and you don't have it. *Install* fetches it. |
| **Error** | The site didn't answer, or it needs a CurseForge key (see below). |

A red note under a name means something looks wrong on disk, for example a
folder with the wrong name (it happens when you unzip a GitHub download as
it is). A grey "built for 12.1.0, game is 12.1.5" is the same *out of date*
warning the game shows, and is usually harmless.

### Updating, and going back

Press **Update** and Tocsin downloads the new version, checks it, and puts it
in place. The version you had is kept aside: if the new one breaks something,
**Rollback** puts the old one back (and you can change your mind again
afterwards).

You can update while the game is open. Type `/reload` in game and the new
version is loaded.

When a site doesn't offer a direct download, *Open* takes you to the page
instead.

### Turning addons off, or removing them

**Disable** moves an addon out of the game's way without deleting it;
**Enable** brings it back. It applies to the whole game folder, not to a
single character, and the addon disappears from the in-game list instead of
showing unticked.

**Remove** takes an addon away, but keeps a copy: it stays at the bottom of
the list with a **Restore** button until you change that addon again. Its
saved settings stay where they are, so reinstalling it later picks them back
up.

### Getting new addons

**Get addons** searches CurseForge, WoWInterface, GitHub and Tukui at the same
time. An addon available on several sites shows up once, with one line per
site, so you can choose where to get it from. Each line shows how popular it
is and when it was last updated, which is handy to spot an old copy someone
forgot on one of them.

Tocsin remembers where you installed an addon from and keeps checking there.
Addons you already have are marked *Installed*. Search runs when you press
Enter.

If an addon you're installing brings along folders you already have (shared
libraries like `Ace3`), Tocsin tells you which and asks before replacing
them.

### More than one game

Every game folder you add (Retail, Classic, …) appears in the sidebar; click
one to switch. In *Settings → Theme* you can choose between a dark look
(*Void*, the default) and a light one (*Sunwell*).

## CurseForge key (optional, but recommended)

GitHub, WoWInterface and Tukui answer anyone. CurseForge only answers people
who have a personal key. It's free and takes a couple of minutes:

1. Go to https://console.curseforge.com/ and sign in.
2. Open *API keys* and copy yours.
3. Paste it in Tocsin under **Settings → API keys**.

Without it, Tocsin still finds a good share of your addons through the other
sites. The ones it can't will say *API key needed*.

You can also add a GitHub token there. You don't need one, it just lets
Tocsin ask GitHub more often.

Keys are kept in Windows' own Credential Manager, never in a file, and are
only sent to the site they belong to.

## Privacy

- Tocsin has no account, no analytics, and no server of its own. It talks
  only to the addon sites listed above.
- When you search, it sends what you typed to GitHub and CurseForge. When it
  checks for updates, it asks each site about the addons that come from it.
  Nothing else about you or your computer is sent.
- It writes only in your `AddOns` folder (the addons, plus three hidden
  folders for downloads in progress, previous versions and disabled addons)
  and in its own cache. It never writes into `WTF`.
- It never installs anything without you pressing a button.

The details, for those who want them, are in
[docs/technical.md](docs/technical.md).

## Something isn't right?

- **An addon is matched to the wrong source, or never updates.** You can tell
  Tocsin where to look, or to skip it. See *Overriding the guess* in
  [docs/technical.md](docs/technical.md).
- **I want to undo everything.** Previous versions live in
  `AddOns\.tocsin-backup`, disabled addons in `AddOns\.tocsin-disabled`.
  Moving a folder from there back into `AddOns` restores it by hand.
- **Found a bug?** Open an issue. For a security problem, see
  [SECURITY.md](SECURITY.md).

## What's next

- Signed Windows releases.
- A preview of what changed before you update.
- Linux and macOS packages.

## For developers

Tocsin is written in Rust. How to build it, how it decides where an addon
comes from, and its security model are in
[docs/technical.md](docs/technical.md). Issues and pull requests are welcome.

## License

MIT, see [LICENSE](LICENSE). The bundled fonts (Cinzel, Alegreya Sans,
JetBrains Mono, in `crates/tocsin/assets/fonts`) are under the SIL Open Font
License 1.1; their licence texts sit next to them. The release zip carries
all of these in its `licenses` folder, together with
`THIRD-PARTY-LICENSES.html`, which lists every open-source library inside
Tocsin and its licence.

World of Warcraft is a trademark of Blizzard Entertainment. This project is
not affiliated with Blizzard, CurseForge, WoWInterface, Tukui or GitHub.
