# Installing and updating addons

Design note for the second milestone: Tocsin downloads a release and puts it in
`Interface/AddOns`, safely, with a way back.

## What other managers do

| | Download | Extract | Replace | Rollback | Game running |
|---|---|---|---|---|---|
| **WowUp** | temp dir in its app data | into a temp dir, then moves the top-level folders into `AddOns` | deletes the folders it recorded at the previous install, then moves the new ones in | none for addon files; "install previous version" re-downloads an older file from the provider; separate WTF backup tool | warns, does not block |
| **CurseForge app** | its own cache | straight into `AddOns`, overwriting | overwrite in place; stale folders from an older layout are left behind | "previous versions" list re-downloads | warns |
| **Ajour** (archived 2024) | temp file | straight into `AddOns` | overwrite, removes folders it knew about | whole-`AddOns`/`WTF` zip backups on a schedule, not per addon | warns |

Two lessons. First, every manager that extracts straight into `AddOns` ends
up with half-written folders when a zip is corrupt or the disk is full.
Second, none of them can undo a single update: the only "rollback" is
downloading an older file again, which depends on the provider still
serving it (GitHub yes, CurseForge usually, Tukui no).

## What Tocsin does

The unit of work is the **release zip**: a WeakAuras zip is five folders,
BigWigs is eleven, ElvUI three. Installing it replaces every folder that the
zip contains, and only those. A family is not always one zip — Routine and
RoutineData are one bundle but two releases — so the install log keeps one
record per zip, keyed by the folder the user acted on, and an action on any
folder of a record (update from a module's row, rollback) resolves to that
record. Keying by family head was the first version of this and it made a
RoutineData update evict `MidnightRoutine` as a folder "the new zip no longer
ships".

### 1. Download

- Into `<cache>/downloads/<provider>-<id>-<version>.zip`, where `<cache>` is
  the per-user cache dir (`%LOCALAPPDATA%\tocsin\cache` on Windows).
  Never inside the game folder.
- Through `http::Http` with the same policy as every other request, plus a
  size cap (default 256 MiB; Details and ElvUI are ~20 MiB) and streaming to
  disk — a release zip never lives whole in memory.
- URL must be `https://`; the host must be the provider's own or its CDN,
  as listed in `download::ALLOWED_HOSTS` (GitHub and its asset hosts,
  `edge.forgecdn.net`, `mediafilez.forgecdn.net`, `api.tukui.org`,
  `cdn.wowinterface.com`, `www.wowinterface.com`). Anything else is refused: an
  API answer is untrusted input and must not turn Tocsin into a downloader
  for arbitrary hosts. The same list applies to every redirect hop, decided
  before the hop is requested (`http::redirect_verdict`): GitHub sends
  `github.com/…/releases/download/…` to `release-assets.githubusercontent.com`,
  which is on the list; a CDN redirecting anywhere else fails the download.
- When the provider gives a hash (CurseForge: `hashes[]`, sha1/md5;
  WoWInterface: `UIMD5`; GitHub: the asset's sha256), verify
  it, computed while the file streams (no second read of the zip). Otherwise
  record our own sha256 of the file for the install log.
- The zip is deleted once the install has run, success or not; Rollback
  never needs it. Files left in the cache for more than a week (a cancelled
  install, a crash) are pruned at start-up.

### 2. Inspect the zip before touching anything

- Open with the `zip` crate, no decompression yet.
- Refuse the archive if any entry: has an absolute path, contains `..`, a
  drive letter or `:`, a Windows device name (`CON`, `NUL`, `aux.lua`, …),
  a name ending in a dot or space or a character Windows forbids, is a
  symlink, or would expand past 2 GiB total / 20 000 files (zip bombs).
  Top-level dot-folders are skipped: a zip shipping `.tocsin-backup/` must not
  replace the backups or the install log.
- Sizes in the zip's directory are the archive's own claim. They are a first
  filter only; extraction counts the bytes really written, per file (never
  more than declared) and in total, and reads each `.toc` through a 1 MiB
  limit.
- Determine the **top-level folders**. A valid WoW zip has only folders at
  the root, each containing `<Folder>/<Folder>*.toc`. A zip with a single
  wrapper folder (`Addon-main/Addon/Addon.toc`, the GitHub source zip) is
  rejected with a clear message rather than guessed at: we never install
  what the author did not package.
- Each top-level folder's `.toc` is parsed with the same parser as the scan,
  for the client flavor. If none of them supports the client flavor
  (`Compat::OtherFlavor` everywhere), stop: the file is for another game.

### 3. Stage

- Extract into `<AddOns>/.tocsin-staging/<family>-<nanos>/`. Same filesystem
  as the destination, so the final step is a rename, not a copy.
- The staging dir name starts with a dot and the scanner skips it, so a
  crash mid-extract leaves nothing the app or the game would mistake for an
  addon. Stale staging dirs are removed on the next launch.
- Extraction honours the entry checks above a second time (paths are
  joined with `Path::join` only after rejecting anything non-normal).

### 4. Swap

Per top-level folder, in order:

1. If `<AddOns>/<Folder>` exists, rename it to
   `<AddOns>/.tocsin-backup/<Folder>@<installed-version>/`.
2. Rename `<staging>/<Folder>` to `<AddOns>/<Folder>`.

Renames on the same volume are atomic per folder. If step 2 fails after
step 1, the backup is renamed back before reporting the error — the user
never ends up with a missing addon. Folders that belonged to the previous
install but are not in the new zip (recorded in the install log) are moved
to the backup too, so an addon that drops a module leaves no stale folder.

### 5. Record

`AddOns/.tocsin-backup/installs.toml` keeps, per release zip (the log moves
with the game folder; see §10 for why that makes it untrusted input):

```toml
[installs.WeakAuras]
provider = "github"
version = "5.22.1"
folders = ["WeakAuras", "WeakAurasOptions", "WeakAurasArchive", "WeakAurasModelPaths", "WeakAurasTemplates"]
installed_at = "2026-10-22T10:14:00Z"
sha256 = "…"
previous = { version = "5.22.0", backup = ".tocsin-backup/…", folders = [...] }
```

This is what makes "which folders are mine to remove" and "what do I roll
back to" deterministic instead of guessed.

### 6. Rollback

One previous version per family, kept as the renamed folders in
`.tocsin-backup` (not the zip, which is deleted after installing).
**Rollback** swaps the current folders out and the backup folders in, same
mechanics as step 4, and updates the record. Keeping exactly one generation
bounds disk use; the backup is deleted when the next update succeeds. (A
"Clear backups" button in Settings is planned, not built.) A user who wants an older-than-previous version
uses "install version…" from the provider's release list, which is step 1
again.

Why folders and not a zip of the old state: a rename is instant and needs no
free space; zipping 11 BigWigs folders on every update would make updates
slow for nothing.

### 7. Game running

`SavedVariables` are written by the game at logout and `/reload`, and Lua
files are read at login. Replacing addon folders while the game runs is
therefore safe for the files Tocsin touches, but the new code only takes
effect after a `/reload`. Tocsin does not block, and does not yet detect a
running client (by process name or a lock on `WTF/`); the README tells the
user to `/reload`. Blocking would be the right call only for `WTF` edits,
which Tocsin never makes.

### 8. Disable, enable, remove

The game loads only what sits directly in `AddOns`, so a disabled addon is a
family renamed into `AddOns/.tocsin-disabled/`. No `WTF` edit: the in-game
toggle lives in per-character `AddOns.txt` files the game rewrites at logout,
which is both racy and N files per addon. The cost is honesty in the README:
disabling is per game folder and the addon vanishes from the in-game list.
Disabled addons are still scanned (flagged) and checked; updating one
re-enables it first so the old copy is backed up like any other.

Remove is "make it the previous generation": folders go to the backup, the
record's current generation is empty, Rollback restores. One generation, as
everywhere else.

### 9. Fresh installs from search

The download and the swap are two steps. After the download, the zip's
folders are compared with what is on disk: many WoWInterface packages ship
libraries such as `Ace3`, `!BugGrabber` or `LibStub` as top-level folders,
and installing them would silently replace a newer standalone copy or a
folder of another addon. When any folder already exists the GUI names them
and asks; *Cancel* deletes the download, *Replace* goes on (the replaced
folders are backed up like any update).

An addon found by search has no folder yet to key its record by, so the
request carries the addon's name instead and the record is keyed by the
zip's *main folder*: the one named like the addon (`ElvUI`), else the one the
other folders depend on (`DBM-Core`), else the shortest. Everything else is
the update path: same download checks, same swap, and any folder already on
disk under that name goes to the backup, so Rollback undoes the install.

The record also keeps the source the user picked. A WoWInterface upload need
not carry `X-WoWI-ID`, and a user who chose WoWInterface over CurseForge
should get its updates; at scan time the recorded source goes first in each of the
record's folders, ahead of what the `.toc` advertises (a `tocsin.toml`
override still wins).

### 10. The install log is input, too

Everything above trusts `installs.toml` to say which folders are Tocsin's and
where their backups are, and acts on it with renames and `remove_dir_all`.
The file sits in the game folder, so it is treated as untrusted on load:
size-capped, and any record whose key or folders are not plain folder names,
or whose backup is anything but `.tocsin-backup/<plain name>`, is dropped
with a warning. A tampered `backup = "C:\\Users\\…"` (which `Path::join`
would happily turn into an absolute path) therefore never reaches a delete.
The three dot-folders are checked with `symlink_metadata` before each use: a
link or junction in their place is refused, never written through.

## What is out of scope for now

- Multiple backup generations.
- Deleting an addon's `SavedVariables`.

## Security notes

Everything above adds exactly one writable location inside the game folder
(`AddOns` and its three dot-folders: staging, backup, disabled) and one cache
dir in the user profile (`downloads/` for release zips, `icons/` for the
library's addon icons). Writes outside those two roots are a bug. Path checks are centralised in one
function with a test for every known traversal trick; `zip` is used with
`enclosed_name()` only.
