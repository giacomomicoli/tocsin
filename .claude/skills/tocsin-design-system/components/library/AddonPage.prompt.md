The addon page: what an addon's site says about it, opened from its icon and name in a Library row or a Get addons result. It takes the content column; the sidebar stays as it is, with the list it came from still lit in the nav.

In the app: `app/page.rs` (`Screen::Addon`, state `AddonView`). The page and its pictures come from `tocsin_core::page`, the description already parsed into blocks, the pictures decoded and scaled. The view only draws.

Layout, top to bottom:

- **Page head** (`widgets::page_head`). The title is the site's name for the addon. The mono subtitle is "CurseForge · by Terciob". On the right: ghost **Back to Library** / **Back to Get addons**, ghost **Open** (the page on the site) and, for a search result, the result's own **Install** (or the **Installed** / **Site only** badge), with the same rules as its source line.
- **Source chips** (`widgets::chip`), one per site with a page, shown only when there are two or more: what each site says can differ.
- **Body**, a themed scroll (`widgets::scroll`). Its column is at most `space::READ_W` (840) wide, with `space::PROSE_GAP` (10) between blocks:
  - the intro: the 32px icon slot and the site's summary (15px, `--text-1`);
  - the **gallery**, when the site has screenshots: the one shown is in a `theme::sunken` well `space::SHOT_H` (360) tall, scaled to fit, and opens the full picture in the browser. Its caption is a muted 12px line. Under it the thumbnails go in a wrapping row: `space::THUMB_H` (54) tall, padding `space::THUMB_PAD`, style `theme::thumb(selected)` (the sunken fill, a row border that turns gold under the cursor, the selected border while shown);
  - the **description**, one block a line:
    - headings in Cinzel: levels 1–2 at 16px `--text-title`, below that 15px `--text-gold`;
    - paragraphs at 14px (`size::TEXT`) in `--text-1`. Bold is the medium face, inline code JetBrains Mono at 12px, links `--text-gold` and underlined. Alegreya Sans ships no italic, so emphasis reads as plain text;
    - list items indented `space::CHILD_INDENT` per level, after a gold `◆` (11px) or a muted "3.";
    - quotes in a `theme::row_child` box, muted;
    - code blocks in a `theme::sunken` box, mono 12px;
    - pictures at their size, scaled down to the column. While one loads, its place is kept: a `theme::sunken` box the width of the column with the loader ("Loading the picture…") centred in it, as tall as the picture will be drawn when its description declares a size (HTML `width` and `height`: decoded to fit the full-picture box, 1280×1600, then scaled down to `space::READ_W`, aspect kept), else `space::PICTURE_H` (120, `--h-picture`). The picture replaces the box when it arrives. One that is not shown (another host, failed, over the page's budget) becomes a `theme::sunken` line "Logo · i.example.com" with a ghost **Open**;
    - rules: the gold rule;
  - "The rest is on the site." with **Open**, when the description was cut to the limits.
- Loading shows the loader ("Loading the page…"). A failure shows its message, muted and centered, with **Add a key** when a CurseForge key would help.

Loading, in order:

- The text shows as soon as the page is in; the pictures follow without holding it.
- Every loader on the page steps (the 250 ms tick runs while the page or any of its pictures loads, and stops after).
- Pictures are asked for in the order the eye meets them: the screenshot shown large, then the thumbnails, then the description's pictures in document order; three at a time.
- Nothing moves when a picture arrives: the large shot's well is `space::SHOT_H` tall from the start, a thumbnail waiting for its picture is an empty 16:9 slot `space::THUMB_H` tall in its `theme::thumb` button (the sunken fill inside the row border), and a description picture keeps the place described above. The placeholders are plain boxes: one background, one border, no motion in the style; only the loader's diamonds step.

The entry point, **NameLink** (`theme::name_link(dim)`): in a Library row and a Get addons result, the icon slot and the name column are one button with no surface or border. Under the cursor the name turns `--text-gold`; the lines with their own tone keep it. `dim` keeps a disabled row's `--text-3`. A row with no page (only a manual link) is the same button, disabled, and looks like the plain row.

Rules:

- Back puts the list back where it was scrolled. Leaving by the nav closes the page, and so does switching installation (back to its list).
- No web view, no page scraping: the description is converted to blocks in the core. A picture is fetched only from `page::media::ALLOWED_HOSTS`; any other is a link.
- Nothing on the page invents a style: the new pieces are `theme::name_link`, `theme::thumb`, `theme::sunken` and the tokens above (`READ_W`, `PROSE_GAP`, `SHOT_H`, `THUMB_H`, `THUMB_PAD`, `PICTURE_H`).

```jsx
<PageHead title="CursedSurges" subtitle="CurseForge · by nero"
  actions={<><Button>Back to Library</Button><Button>Open</Button></>} />
<Chips><Chip selected>CurseForge</Chip><Chip>GitHub</Chip></Chips>
<Scroll maxWidth="var(--w-read)">
  <Intro icon="CS">Countdown, waypoint, and zone announce for Cursed Surge events.</Intro>
  <Gallery shown={0} shots={["raid.png"]} caption="Live in Game!" />
  <p>CursedSurges is a small tracker for <b>Cursed Surge</b> events…</p>
  <li>◆ Countdown to the next surge…</li>
</Scroll>
```
