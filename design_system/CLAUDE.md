# CLAUDE.md

Design system for the AccessMod web client. Read `docs/DESIGN.md` before
writing UI. In short:

- Match AM5: look at `references/screens/<id>/en.png` and `dom.html` for the
  screen you build; `references/inventory.md` maps screens to AM5 sources and
  i18n keys.
- Tokens only (`design-system/src/styles/tokens.css`), both themes.
- No hard-coded text: AM5 keys via `t()` / `<T k>`; new keys in
  `design-system/src/i18n/own.json` (four languages), then `cargo xtask i18n`
  rebuilds `catalog.json` (never edit it by hand).
- Icons through `icons` (`components/am/icons.ts`).
- Compose from `components/am` first.

The package is `@accessmod/ui` in the root npm workspace (`npm ci` at the
root); internal imports use `#ui/*`, never `@/`.

Checks (`npm -w @accessmod/ui run …`): `typecheck`, `contrast`,
`build`; with `dev` running, `visual` and review
`visual/report.html` in light and dark.

The AM5 source is `../_am5/accessmod` (`sh scripts/fetch-am5.sh`, untracked);
`scripts/capture` needs AM5 running at http://localhost:3080 and must stay
read-only toward it.
