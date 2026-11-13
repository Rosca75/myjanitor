# doc-anonymiser palette (reference for MyJanitor tokens)

Source of truth: `frontend/brand.css` (all colour and font family, 150 lines) and `frontend/tokens.css` (all non-colour primitives, 139 lines). `style.css` only consumes them; `docs/docs.css` reuses brand.css. All values below are exact, copied from source. Contrast ratios are WCAG 2.x relative-luminance, computed with a python script.

## 1. Colour tokens (brand.css, every one declared in `:root`)

All source values are hex, so there is no second notation. Colours elsewhere: `tokens.css` has `--elev-raised: 0 6px 18px rgba(0, 0, 0, 0.12)` and `--scrim: rgba(0, 0, 0, 0.28)`; `style.css` has `color-mix(in srgb, var(--warn) 30%, transparent)` (search hit) and 60% (active hit). No other literals.

| Token | Value | Role | Used for |
|---|---|---|---|
| `--bg` | `#FFFFFF` | page, card, modal, input surface | body, topbar, `.card`, `.modal`, inputs, danger-button rest fill |
| `--bg-secondary` | `#EBEBEB` | quiet secondary surface | `.card-head` strip |
| `--text` | `#000000` | all text on light surfaces | body text, button text |
| `--muted` | `#A1A8B3` | decorative secondary (2.40:1 on white, NOT for text) | `.hint`, `.card-sub`, chevrons, chip-remove glyph |
| `--accent` | `#FD5108` | THE one hero orange | primary button, progress fill, active-cell outline |
| `--accent-medium` | `#FE7C39` | accent lighter | focus ring colour |
| `--accent-light` | `#FFAA72` | accent lightest | run-bar progress fill, inactive cell outline |
| `--tint-orange-1` | `#FFCDA8` | orange tint | callout border, key-edge |
| `--tint-orange-2` | `#FFE8D4` | orange tint | key-edge-soft |
| `--tint-orange-3` | `#FFF5ED` | orange tint, lightest | callout / key-bearing fill |
| `--gray-1` | `#A1A8B3` | grey ramp (= muted) | |
| `--gray-2` | `#B5BCC4` | grey ramp | |
| `--gray-3` | `#CBD1D6` | control border | button, input, `.tag` borders |
| `--gray-4` | `#DFE3E6` | hairline border | card, modal, topbar, code borders |
| `--gray-5` | `#EEEFF1` | secondary button fill, track | `.btn-secondary`, hover fill, progress track, `.tag` fill |
| `--gray-6` | `#F5F7F8` | subtle fill | chip-tag, code, banners, notice-info bg |
| `--ink-2` | `#54616C` | sentence-level quiet text (6.36:1 on white) | modal body, Back button fill |
| `--ink-3` | `#717C8D` | label text (4.23:1 on white) | uppercase labels, stat labels |
| `--surface-subtle` | `#F9FAFB` | barely-tinted white | table header rows, group headers |
| `--ok` | `#059669` | success status fill/glyph (3.77:1 on white) | `.icon-action.ok:hover` |
| `--warn` | `#E9B01F` | warning status (1.96:1 on white, border/outline only) | `.tag.warn` and banner borders, find-hit tint |
| `--err` | `#DC2626` | error/destructive (4.83:1 on white) | danger button, error borders, remove hover |
| `--valid-fg` / `--valid-bg` | `#1F5F4A` / `#E3F1EC` | valid regex, High score | `.state-tag`, `.score-high` |
| `--invalid-fg` / `--invalid-bg` | `#9A2020` / `#FBE6E6` | invalid, conflicted row | `.state-tag.bad`, conflicted cards |
| `--notice-ok-fg/-bg/-edge` | `#0A6B3D` / `#F0FBF4` / `#BFE6CE` | success toast | `.notice-ok` |
| `--notice-info-fg/-bg/-edge` | `#54616C` / `#F5F7F8` / `#EEEFF1` | neutral toast, Low score | `.notice`, `.score-low` |
| `--notice-warn-fg/-bg/-edge` | `#6B5310` / `#FBF6E6` / `#F5E3B8` | warning toast | `.notice-warn` |
| `--caution-fg/-bg/-edge` | `#6B5310` / `#FBF6E6` / `#F5E3B8` | in-report warning strip, Medium score | `.caution`, `.score-medium` |
| `--key-fg` | `#B4551A` | key-bearing (re-identification) label, 4.59:1 on `#FFF5ED` | `.card.key-bearing` caption |
| `--key-bg` | `#FFF5ED` | key-bearing fill | key card head, draft row, pending chip |
| `--key-edge` / `--key-edge-soft` | `#FFCDA8` / `#FFE8D4` | key-bearing borders | |
| `--more-chip-fg/-bg/-edge` | `#205FAE` / `#EEF5FF` / `#CFE0FB` | "+N more" chip (the only informational blue) | `.chip-more` |
| `--selected-bg` / `--selected-edge` | `#EEF5FF` / `#CFE0FB` | selected row/card | `.values-value-row.selected` |
| `--origin-fg/-bg/-edge` | `#205FAE` / `#EEF5FF` / `#CFE0FB` | positional "you are here" tint | `.value-origin.is-linked` |
| `--held-fg` | `#66717F` | held-back item text (4.96:1 on white) | not-replaced spellings |
| `--kept-text` | `#555555` | grey prose around replacements | anonymised pane |

Brand rules from the file: orange is ONE hero element per view, never body text, never a flood behind small text; black text on every light fill; statuses are functional only.

## 2. Dark theme

NONE. A grep for `prefers-color-scheme`, `data-theme`, `color-scheme` and `dark` in `frontend/` CSS, HTML and JS finds nothing. Light only, single `:root` block, no switch. Any MyJanitor dark theme would be DERIVED work with no reference values.

## 3. Look tokens

**Typography** (brand.css): `--font-heading` and `--font-body` both `Helvetica, Arial, sans-serif`; `--font-mono` `Consolas, 'Courier New', monospace` (previews, `code`). System fonts only, no font files. Headings weight 400 (hierarchy from size, never bold); `.brand` mark 700; primary button, uppercase labels, table heads and stat labels 700 with letter-spacing 0.06em to 0.1em; docs body line-height 1.55.

| Token (tokens.css) | Value | Use |
|---|---|---|
| `--text-caption` | 0.7rem | counts, units, timestamps |
| `--text-small` | 0.75rem | chip, badge, table cell |
| `--text-body` | 0.8rem | base size of every control |
| `--text-emphasis` | 0.9rem | important value, dialog head, `.hint` |
| `--text-title` | 1rem | prominent label |
| `--text-heading` | 1.2rem | card head |
| `--display-md` | 1.6rem | brand mark |
| `--display-lg` | 2rem | hero, once |

**Spacing**: `--space-tight` 0.3rem, `--space-control` 0.75rem, `--space-card` 1rem, `--space-group` 1.5rem, `--space-workspace` 2.5rem; `--space-hair` 0.15rem. Insets: `--control-inset` 0.3rem, `--chip-inset` 0.2rem, `--badge-inset` 0.1rem; `--control-line` 1.1rem.
**Radii**: `--radius-sm` 4px (chip, badge), `--radius-md` 6px (card, button, input, dialog; `--card-radius`), `--shape-circle` 50%, `--shape-pill` 999px.
**Shadow**: exactly one, `--elev-raised: 0 6px 18px rgba(0, 0, 0, 0.12)` for every floating surface; `--scrim: rgba(0, 0, 0, 0.28)`. Cards are flat (border only).
**Focus ring**: `outline: 2px solid var(--accent-medium)` (`#FE7C39`) with `outline-offset: 1px` on button, `.btn`, input, select (`:focus-visible`); marks and warn icons use `--accent` `#FD5108`; grid cell outline `--accent-light`, focused `--accent`.
**Transitions**: `transform 0.15s` (chevron, sort arrow), `border-color 0.12s, background 0.12s`, `background 0.12s`, `width 0.2s` (progress), `background-color 90ms ease-in`. Default easing.
**Layout**: header 5rem, step bar 4.25rem, footer 3.75rem, strip 2.6rem, modal width 27.5rem. Layers 1/6/30/55/60.

## 4. Component conventions

- **Panel/card**: `#FFFFFF` fill, `1px solid #DFE3E6`, radius 6px, overflow hidden; head strip 2.6rem filled `#EBEBEB` with `#EEEFF1` bottom hairline. Key-bearing card: border `#FFCDA8`, head `#FFF5ED`. Modal: same look plus `--elev-raised` over `--scrim`.
- **Button (base/secondary)**: fill `#EEEFF1`, border `1px #CBD1D6`, radius 6px, black text, padding 0.3rem 0.75rem, 0.8rem; disabled opacity 0.5. Ghost: transparent, hover `#EEEFF1`.
- **Primary**: fill and border `#FD5108`, text `#FFFFFF` bold (3.30:1, the source allows it only as bold large text). Step Back: `#54616C` fill, white text, hover `#000000`.
- **Danger**: quiet at rest (white fill, `#DC2626` border and bold text), hover fills `#DC2626` with white text.
- **Input/select**: white, `1px #CBD1D6`, radius 6px, same height as a button; focus ring as above.
- **Badge/pill/chip**: `.tag` uppercase bold 0.75rem, `#EEEFF1` fill, `#CBD1D6` border, radius 4px; `.state-tag` and `.score-chip` radius 4px, fg/bg pair, no border (High `#1F5F4A`/`#E3F1EC`, Medium `#6B5310`/`#FBF6E6` with `#F5E3B8` edge, Low `#54616C`/`#F5F7F8` with `#EEEFF1` edge); `.chip-tag` `#F5F7F8` fill, `#EEEFF1` border, radius 6px.
- **Banner/notice**: tinted fill, dark fg, 1px top edge in the `-edge` colour (notice) or a full border in `--warn`/`--err` on `#F5F7F8` (banner).
- **Progress bar**: 6px, track `#EEEFF1`, fill `#FD5108`, radius 4px.

## 5. Proposed MyJanitor mapping

Ratios are foreground against the named background (WCAG 2.x). Only `--accent-text` is DERIVED (a new value); every other row re-assigns an existing reference value to a role, with the ratio verified. Roles the reference has no named token for (hover, on-accent, raised, sunken, border-strong, text-faint) are marked "re-assigned".

| Role | Value | Source / status | Contrast |
|---|---|---|---|
| `--accent` | `#FD5108` | `--accent` | fill only; vs `#FFFFFF` 3.30:1 (non-text UI use) |
| `--accent-hover` | `#FE7C39` | `--accent-medium`, re-assigned | `#000000` on it 8.16:1 |
| `--accent-soft` | `#FFF5ED` | `--tint-orange-3` (stronger: `#FFE8D4`) | |
| `--on-accent` | `#000000` | source rule "black on light fills"; white-bold is the source's exception | on `#FD5108` 6.36:1 (white: 3.30:1) |
| `--accent-text` (extra) | `#A93600` | DERIVED, orange for text/links | on `#FFFFFF` 6.53:1, on `#FFE8D4` 5.52:1 |
| `--success` | `#059669` | `--ok` (icons, bars, fills) | vs white 3.77:1 |
| `--success-soft` | `#F0FBF4` | `--notice-ok-bg` | `#0A6B3D` on it 6.22:1 |
| `--success-text` (extra) | `#0A6B3D` | `--notice-ok-fg` | on white 6.60:1 |
| `--warning` | `#E9B01F` | `--warn` (fill, border, icon; never text) | vs white 1.96:1; black on it 10.69:1 |
| `--warning-soft` | `#FBF6E6` | `--caution-bg` | `#6B5310` on it 6.77:1 |
| `--warning-text` (extra) | `#6B5310` | `--caution-fg` | on white 7.32:1 |
| `--danger` | `#DC2626` | `--err` | white on it 4.83:1; it on white 4.83:1 |
| `--danger-soft` | `#FBE6E6` | `--invalid-bg` | `#9A2020` on it 6.75:1 |
| `--danger-text` (extra) | `#9A2020` | `--invalid-fg` | on white 8.07:1 |
| `--info` | `#205FAE` | `--more-chip-fg` | on white 6.35:1 |
| `--info-soft` | `#EEF5FF` | `--selected-bg` / `--origin-bg` | `#205FAE` on it 5.78:1 |
| `--surface` | `#FFFFFF` | `--bg` | |
| `--surface-raised` | `#F9FAFB` | `--surface-subtle`, re-assigned (card heads, table headers; floating layers stay `#FFFFFF` + `--elev-raised`) | black 20.1:1; `#54616C` 6.08:1 |
| `--surface-sunken` | `#F5F7F8` | `--gray-6`, re-assigned (wells, code, chips; `#EBEBEB` for strips) | black 19.54:1; `#54616C` 5.92:1 |
| `--border` | `#DFE3E6` | `--gray-4` | hairline, 1.29:1 on white |
| `--border-strong` | `#CBD1D6` | `--gray-3` (control borders) | 1.54:1 on white; never the sole cue |
| `--text` | `#000000` | `--text` | 21.0:1 on white, 17.62:1 on `#EBEBEB` |
| `--text-muted` | `#54616C` | `--ink-2` | 6.36:1 on white, 5.92:1 on `#F5F7F8` |
| `--text-faint` | `#66717F` | `--held-fg`, re-assigned (`--ink-3` 4.23:1 and `--muted` 2.40:1 fail 4.5:1) | 4.96:1 on white, 4.75:1 on `#F9FAFB`, 4.62:1 on `#F5F7F8` |

Notes:
- "In plan" / bulk selection: use `--info-soft` fill with `#CFE0FB` edge (`--selected-edge`), or `--accent-soft` with `#FFCDA8`; the reference reserves orange for ONE hero element per view, so prefer blue for selection and orange for the primary action.
- Lossy/irreversible: `--warning-soft` + `--warning-text`. Permanent delete: the quiet danger button (white, red border) that fills `--danger` on hover.
- No dark theme exists in the reference; all ratios hold on light surfaces only.
