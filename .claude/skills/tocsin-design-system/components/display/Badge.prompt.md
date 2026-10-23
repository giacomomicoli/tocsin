Status badge on an addon row — always the core's UpdateStatus label text, colored by status.

```jsx
<Badge status="update">Update available</Badge>
<Badge status="ok">Up to date</Badge>
<Badge status="error">Missing</Badge>
```

"Missing" is not an `UpdateStatus`: the library draws it for an absent dependency, with the error tone (`badge("Missing", Tone::Error)`).

Rendered uppercase in Cinzel 11px. Sits right-aligned in a 150px column.
