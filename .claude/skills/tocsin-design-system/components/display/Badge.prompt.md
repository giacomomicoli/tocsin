Status badge on an addon row — always the core's UpdateStatus label text, colored by status.

```jsx
<Badge status="update">Update available</Badge>
<Badge status="ok">Up to date</Badge>
<Badge status="error">Missing</Badge>
```

"Missing" is not an `UpdateStatus`: the library draws it for an absent dependency, with the error tone (`badge("Missing", Tone::Error)`).

Get addons uses two more, neither of them an `UpdateStatus`:

- "Installed" (`Tone::Ok`), at the top of a result's sources column, when the addon is on disk already.
- "Site only" (`Tone::Manual`), in place of a source line's Install button, when that site has nothing an app can download. This happens on a CurseForge project with `allowModDistribution: false`, or on a GitHub repository found to have no release. Open, next to it, is the way in.

Each source line of a search result also carries a fit badge: what that site says about the selected game client. Its label is `Compat::label(flavor)` (core), never typed in the view, and its tone comes from the fit:

| Fit (`Compat`) | Label | Tone |
|---|---|---|
| `Exact`: the site lists a file for this build | the flavor: "Retail", "Classic Era", "Classic", "Forever" | `Tone::Ok` |
| `Older`: listed for an older build of this game | "Out of date" | `Tone::Update` |
| `Newer`: listed for a newer build | "Newer build" | `Tone::Ahead` |
| `Unknown`: the site does not say (GitHub always, many WoWInterface files) | "Unverified" | `Tone::Manual` |

(`OtherFlavor`, "Other game", `Tone::Error`, should never show: the search drops those hits.) It sits after the facts and before the line's HelpTip "?", which opens `Compat::describe(..)`; then Open, then Install or "Site only". The line is `provider 96 · facts · fit · ? · Open · Install` in the sources column (`--w-sources`), and the facts are the part cut at the edge when room runs out. Installed results show the fit badge too, without Install.

Rendered uppercase in Cinzel 11px. In the library it sits right-aligned in a 150px column. In a search result it takes the size of its text.
