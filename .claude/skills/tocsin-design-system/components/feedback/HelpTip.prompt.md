HelpTip: a small "?" placed right after a badge whose reason isn't obvious. Clicking it opens a box underneath that explains the badge, and clicking again closes it. iced has no popover that opens on click (its `tooltip` only appears on hover), so the box is laid out in the flow, under the line it explains. It is not drawn floating over the page.

In the app:

- `widgets::help(open, message)`: the "?" in Cinzel 11, padding `space::HELP`, style `theme::help(open)`. It takes the manual tone of the badge next to it: a 50% border, filled 20% while open or hovered.
- `widgets::tip(body)`: the explanation at 12px in `--text-1`, padding `space::TIP_PAD`, style `theme::tip`. That is `--surface-sunken` with a border in the manual tone at 50%, r2. It fills the width of the column it sits in.

Rules:

- Only one tip is open at a time. A new search closes it.
- The explanation says why the badge is there and what to do about it, in two or three plain sentences. Write it in the README voice and leave out URLs and internals.
- Use it only where a status needs a reason the user can't guess. Never use it as decoration or to hold help text.

Used by:

- Get addons: next to "Site only", to say why that site can't be installed from in Tocsin. The cause is either a CurseForge author's choice or a GitHub repository with no releases.

```jsx
<span class="badge manual">SITE ONLY</span> <HelpButton open={open} onClick={toggle}>?</HelpButton>
{open && <Tip>The author lets CurseForge hand the files out only on its website and in its own app, so Tocsin gets no download link. …</Tip>}
```
