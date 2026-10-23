Tocsin's button — use primary for the single main action in an area (Check updates, Update, Install), secondary for everything else, danger for Remove, selected for the active nav or filter chip.

```jsx
<Button variant="primary" size="md">Check updates</Button>
<Button size="sm">Open</Button>
<Button variant="danger" size="sm">Remove</Button>
<Button variant="selected" size="chip">All</Button>
```

- Primary/selected labels use Cinzel; secondary/danger use Alegreya Sans.
- Busy states change the label ("Working…", "Installing…") and disable — no spinners inside buttons.
