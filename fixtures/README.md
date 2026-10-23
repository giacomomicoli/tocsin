# Local test fixtures

`fixtures/AddOns/` is a stand-in for a real `Interface/AddOns` folder, used to
try Tocsin without touching a game install:

```sh
cargo run -p tocsin-core --example check -- fixtures/AddOns
```

It is git-ignored because it holds third-party addon files. To populate it,
copy whole addon folders from your game, or just the files Tocsin reads:
`<Addon>.toc`, plus `CHANGELOG.md`/`README.md` and `Ver.lua` when present.

The addons used while developing cover the interesting cases:

- Routine + Routine Data: GitHub link only in the changelog, two addons
  released from one repository under different tag prefixes, a dependency
  between them.
- Zygor Guides Viewer: version split between the `.toc` and `Ver.lua`, a
  source with no API at all.
- BigWigs: ten `LoadOnDemand` modules with no `## Author` and no ids,
  `X-Website` pointing at a GitHub *organisation*, three ids in the head.
- WeakAuras: five folders from one zip, `Globe-Main` grouping, a package
  shipped without a Retail `.toc`.
- ElvUI: Tukui source with a negative project id, six conditional `## Title`
  lines (`[AllowLoadGameType …]`), `X-Tukui-ProjectFolders`.
- Details: a GitHub "Source code" zip extracted as-is — misnamed folder,
  `.pkgmeta`, no version.
- HandyNotes: head of a family whose plugins come from other authors.
- Ace3: a library with `Release-r1403` as version and a GitHub mirror
  without releases.
