Feedback primitives: Notice for the footer status line, ConfirmBanner for a question that blocks a destructive action (or a large download), Progress for a running download, Loader for any wait.

Loader is `App::loader(label)` in the app (the dots then a muted 12px label), or `App::loader_dots()` alone. It turns on the 250ms tick, which runs only while something waits. Used by:

- Get addons: the catalog download and a search ("Searching…"), centred in the list area.
- Library, first scan: "Scanning Retail…" (the installation's label) centred in the empty list.
- Addon page: the page and its pictures loading.
- The footer, while the app is busy (a scan, a check, an install, a move): the dots alone, before the notice. During a check the notice is its live count, "Checking updates · 12 of 55…", with no "×"; the summary ("3 updates available · …") replaces it when the last answer arrives. While a check waits for the API keys: "Reading your API keys…". Each row's badge reads "Checking…" until its own answer arrives.

ConfirmBanner is `widgets::confirm_banner(title, body, actions)` in the app (padding `space::CONFIRM_PAD`, style `theme::confirm`). It sits under the page's head and toolbar, above the list. Title is a question ("Replace 2 folders?", "Delete Bagnon?"); body says what happens and what cannot be undone; actions are ghost Cancel then danger for the destructive answer. Used by:

- Get addons: an install whose zip replaces folders already in AddOns (Replace).
- Library: Delete on a disabled or removed addon (Delete) — the one action nothing restores.
- Library: Move up on a wrapped row whose addons are already installed (Replace): "Replace 1 folder?", body naming them and saying Rollback brings them back.
- Any page: the Library's "Remove folder", the sidebar's "×" or Settings' Remove on a game folder ("Remove Retail from Tocsin?", Remove). Nothing on disk changes, but the mark is small and sits on the item it would switch to, so it asks; the body names the full path, since two folders can share a flavor name. Shown above the page, on whichever page is open.
- Get addons (or above any page, for a missing dependency's Install): a fresh install whose zip is larger than the usual 256 MB cap, a voice-over pack ("Download 453 MB?"), asked before a byte of it is fetched (`download::is_large`). The body names the addon, the site and the size, and says it needs about twice that on disk for a moment. Nothing is replaced, so the answer is the **primary** button "Download", after ghost Cancel; not danger. Updates of an addon already installed never ask.

Progress is `App::progress` in the app: beside the footer notice, on its right before the "×", while a download runs. A mono 12px muted label ("113 MB of 453 MB", from `download::Progress::label`) and, when the size is known, a bar `progress_bar(0..=1, fraction)` sized `space::PROGRESS_W` × `space::PROGRESS_H` (160 × 6, `--w-progress` / `--h-progress`), style `theme::progress`: `--surface-sunken` rail, 1px `--border-frame`, r2, filled with `--accent`. No new color. While the size is unknown, the label alone (no empty bar). It stays when the notice is dismissed, and goes when the download ends; the notice reads "Downloading … · 453 MB…", then "Installing …" once the zip is verified. The view reads a copy taken on each tick (`Transfer`), never the shared counters.

```jsx
<Notice onDismiss={close}>3 updates found.</Notice>
<ConfirmBanner title="Replace 2 folders?" actions={<><Button>Cancel</Button><Button variant="danger">Replace</Button></>}>
  The zip ships Ace3 and LibStub, which you already have.
</ConfirmBanner>
<ConfirmBanner title="Delete Bagnon?" actions={<><Button>Cancel</Button><Button variant="danger">Delete</Button></>}>
  Bagnon and its kept copy are deleted from AddOns. This cannot be undone; its saved settings stay.
</ConfirmBanner>
<ConfirmBanner title="Download 453 MB?" actions={<><Button>Cancel</Button><Button variant="primary">Download</Button></>}>
  Spoken Audio: Readables from CurseForge is a 453 MB zip, far larger than most addons.
</ConfirmBanner>
<Notice onDismiss={close}>Downloading Spoken Audio: Readables from CurseForge · 453 MB…</Notice>
<Progress value={0.25} label="113 MB of 453 MB" />
<Loader lit={tick % 3} />
```

Note: loader diamonds are 45°-rotated squares here; in iced (no container rotation) use a "◆" glyph or a small image.
