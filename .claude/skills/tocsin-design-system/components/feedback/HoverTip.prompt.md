HoverTip: the whole of a mono value that was cut to fit its fixed column. The value shows as many characters as fit, the last one `…`; resting the pointer on it opens a small box just above with the full text. It uses iced's `tooltip`, which opens on hover only, so it is never the only way to reach something the user needs to act on.

In the app:

- `widgets::mono_fitted(value, size, width, styled)`: the value in JetBrains Mono at `size`, cut with `widgets::ellipsize` to `theme::mono_fit(width, size)` characters. Every glyph of the mono font is `theme::MONO_ADVANCE` (0.6 em) wide, so the count is exact; the clipped container is the backstop. When nothing was cut, no tip is attached. `styled` adds the color and leaves it out to keep the row's own (faded on disabled rows).
- The tip: the full value in mono 12px, `--text-1`, padding `space::TIP_PAD`, style `theme::hover_tip`. That is `--surface-sunken` with a 1px `--border-frame`, r2, and the frame's drop shadow, because it floats over the rows. It sits `space::HOVER_GAP` (4px) above the value.

Rules:

- Only for values in a fixed-width column that can be longer than it: versions, file names, paths. Text that can wrap wraps instead.
- The tip repeats the value; it never explains it. A reason the user can't guess goes in a HelpTip.

Used by:

- Library: both lines of the versions column (installed, latest), 120px. A CurseForge file name standing in for a version (`AztarecHelper-2.9.0.zip`) or a fallback provider's `· WoWInterface` suffix no longer runs into the badge.

```jsx
<span class="mono" title="AztarecHelper-2.9.0.zip">AztarecHelper-2…</span>
<Tip hover>AztarecHelper-2.9.0.zip</Tip>
```
