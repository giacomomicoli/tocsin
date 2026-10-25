The library's addon row — use for every line in the installed list, including family members, missing dependencies and removed families.

```jsx
<AddonRow name="Routine" meta="MidnightRoutine · GitHub" installed="v12.0.376" latest="v12.0.381"
  status="update" statusLabel="Update available"
  actions={<><Button>Open</Button><Button variant="primary">Update</Button><Button>Disable</Button><Button variant="danger">Remove</Button></>} />
<AddonRow child name="Routine Data" meta="RoutineData · GitHub" installed="dv12.0.13" latest="dv12.0.13" status="ok" statusLabel="Up to date" />
<AddonRow kind="missing" name="LibDataBroker-1.1" meta="required by HandyNotes · not installed" actions={<Button variant="primary">Install</Button>} />
```

Columns: name (fill) · versions 120px right-aligned mono · badge 150px right-aligned · actions. Children get no Disable/Remove (they follow the head).

A disabled head adds a danger **Delete** after Remove (Open · Enable · Remove · Delete); a removed row shows ghost **Restore** then danger **Delete**. Delete never acts at once: it raises the ConfirmBanner (`feedback/Notice`) above the list.
