Gilded frame for a section that needs its own surface. In the app it holds the Settings sections (Game Folders, API Keys, Theme); GoldRule divides sections inside it.

```jsx
<Frame>
  <Wordmark />
  <GoldRule style={{ margin: "10px 0" }} />
  …
</Frame>
```

Use at most one or two per screen. Rows are not framed — they use plain row surfaces.
