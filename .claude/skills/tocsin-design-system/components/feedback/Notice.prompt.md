Feedback primitives: Notice for the footer status line, ConfirmBanner for a question that blocks an install, Loader for catalog/search waits.

```jsx
<Notice onDismiss={close}>3 updates found.</Notice>
<ConfirmBanner title="Replace 2 folders?" actions={<><Button variant="primary">Replace</Button><Button>Cancel</Button></>}>
  The zip ships Ace3 and LibStub, which you already have.
</ConfirmBanner>
<Loader lit={tick % 3} />
```

Note: loader diamonds are 45°-rotated squares here; in iced (no container rotation) use a "◆" glyph or a small image.
