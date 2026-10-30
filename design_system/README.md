# AccessMod design system

Visual reference and component library for the AccessMod web client. It reproduces AccessMod 5 (Shiny, Bootstrap 3,
AdminLTE) with React, Tailwind v4 and shadcn/ui, adds a dark theme, and keeps
the AM5 i18n keys (en, fr, es, pt).

| Folder | Content |
|---|---|
| `references/` | AM5 5.9.2 captured from a running instance: 29 screens × 4 languages, DOM, ARIA, i18n keys, measured styles, component crops, `inventory.md` |
| `design-system/` | tokens, components, i18n catalogue, gallery with AM5 overlay, visual report, contrast check |
| `scripts/capture/` | Playwright capture of AM5 (repeatable) |
| `docs/` | `DESIGN.md` (rules and catalogue), `icons.md` |
| `../_am5/accessmod` | AM5 5.9.2-beta.6 source (`sh scripts/fetch-am5.sh`, not committed) |

Start with `docs/DESIGN.md`.
