Tocsin's button — use primary for the single main action in an area (Check updates, Update, Install), secondary for everything else, danger for Remove, selected for the active nav or filter chip.

```jsx
<Button variant="primary" size="md">Check updates</Button>
<Button size="sm">Open</Button>
<Button variant="danger" size="sm">Remove</Button>
<Button variant="selected" size="chip">All</Button>
```

- Primary/selected labels use Cinzel; secondary/danger use Alegreya Sans.
- A button that only raises a ConfirmBanner is secondary even when the action is destructive ("Remove folder" in the Library head): the banner's answer is the danger one. A Remove that belongs to a row (a game folder in Settings, a stored key) is danger.
- Remove mark (`theme::remove_mark`): a "×" (Alegreya Sans 12, padding `space::HELP`, r2) in the top-right corner of a sidebar installation, `space::CORNER` from its edge, shown only while the item is hovered (iced `hover`) and never while busy. At rest a bare `--text-2` glyph; under the cursor it takes the danger look. It never acts on its own: it raises the ConfirmBanner.
- Busy states change the label ("Working…", "Installing…") and disable — no spinners inside buttons.
