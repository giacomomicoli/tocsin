The library's addon row — use for every line in the installed list, including family members, missing dependencies and removed families.

```jsx
<AddonRow name="Routine" meta="MidnightRoutine · GitHub" installed="v12.0.376" latest="v12.0.381"
  status="update" statusLabel="Update available"
  actions={<><Button>Open</Button><Button variant="primary">Update</Button><Button>Disable</Button><Button variant="danger">Remove</Button></>} />
<AddonRow child name="Routine Data" meta="RoutineData · GitHub" installed="dv12.0.13" latest="dv12.0.13" status="ok" statusLabel="Up to date" />
<AddonRow kind="missing" name="LibDataBroker-1.1" meta="required by HandyNotes · not installed" actions={<Button variant="primary">Install</Button>} />
```

Columns: name (fill) · versions 120px right-aligned mono, cut with `…` to fit and whole in a HoverTip (`widgets::mono_fitted`) · badge 150px right-aligned · actions. Children get no Disable/Remove (they follow the head).

```jsx
<AddonRow kind="wrapped" name="Details-Details.20261104.15400.172-1-gcec582c"
  meta="8 addons one level too deep · the game does not load them"
  notes={[{ text: "Details, Details_Compare2, Details_DataStorage, …", tone: "muted" }]}
  actions={<Button variant="primary">Move up</Button>} />
```

A **wrapped** row is a folder in AddOns that holds addon folders one level too deep (a zip unpacked with Windows' "Extract all"): the missing row's look (transparent, red-tinted border, dimmed 32px icon slot) at head level, no versions column, the meta line in the error tone, the inner folders in a muted line, badge **Not loaded** (`Tone::Error`), one primary **Move up**. Wrapped rows come first in the list, under every filter but Updates. When the move would replace folders already there, Move up raises the ConfirmBanner ("Replace N folders?") instead of acting.

A disabled head adds a danger **Delete** after Remove (Open · Enable · Remove · Delete); a removed row shows ghost **Restore** then danger **Delete**. Delete never acts at once: it raises the ConfirmBanner (`feedback/Notice`) above the list.
