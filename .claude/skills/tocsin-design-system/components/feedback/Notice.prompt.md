Feedback primitives: Notice for the footer status line, ConfirmBanner for a question that blocks a destructive action, Loader for catalog/search waits.

ConfirmBanner is `widgets::confirm_banner(title, body, actions)` in the app (padding `space::CONFIRM_PAD`, style `theme::confirm`). It sits under the page's head and toolbar, above the list. Title is a question ("Replace 2 folders?", "Delete Bagnon?"); body says what happens and what cannot be undone; actions are ghost Cancel then danger for the destructive answer. Used by:

- Get addons: an install whose zip replaces folders already in AddOns (Replace).
- Library: Delete on a disabled or removed addon (Delete) — the one action nothing restores.

```jsx
<Notice onDismiss={close}>3 updates found.</Notice>
<ConfirmBanner title="Replace 2 folders?" actions={<><Button>Cancel</Button><Button variant="danger">Replace</Button></>}>
  The zip ships Ace3 and LibStub, which you already have.
</ConfirmBanner>
<ConfirmBanner title="Delete Bagnon?" actions={<><Button>Cancel</Button><Button variant="danger">Delete</Button></>}>
  Bagnon and its kept copy are deleted from AddOns. This cannot be undone; its saved settings stay.
</ConfirmBanner>
<Loader lit={tick % 3} />
```

Note: loader diamonds are 45°-rotated squares here; in iced (no container rotation) use a "◆" glyph or a small image.
