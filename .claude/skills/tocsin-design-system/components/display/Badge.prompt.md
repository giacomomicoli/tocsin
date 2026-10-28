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

Rendered uppercase in Cinzel 11px. In the library it sits right-aligned in a 150px column. In a search result it takes the size of its text.
