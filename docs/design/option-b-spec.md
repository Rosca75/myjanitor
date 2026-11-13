# MyJanitor — Option B "Guided plan": implementation spec

Source: `MyJanitor-wireframes.figcraft` (page "Option B — Guided plan"), `document.json` parsed successfully; PNG renders used for cross-checking. All values are CSS px in frame-local coordinates (frame origin top-left). Font is Inter throughout (wireframe default). Wireframe stroke weights not stored on frames are read as 1px from the renders. Badge text is stored mixed-case but rendered UPPERCASE (`text-transform: uppercase`); copy below gives the stored text, with the rendered form noted.

Flow: scan → ranked opportunity cards → add to plan → review each step → confirm → apply → undo from History.

## 1. Screens and region trees

Both screens are 1440 x 900 frames (white fill, 1px #D1D5DB border).

### B1 Overview (results loaded)
```
Frame 1440x900
├─ Nav rail            x0 y0   84 x 900 (frame shows 856 tall; treat as full height minus strip)
├─ Header              x84 y0  1356 x 64      (bottom border 1px #E5E7EB at y63)
│   ├─ Title "Overview" (18/700) x108
│   ├─ Scope chips (C:\Users\Michel, D:\Projects) + "+ Add location" link
│   └─ right: "Scanned 2 min ago · cached" + [Scan again] button
├─ Center (bg #F9FAFB) x84 y64  996 x 792, content inset 24px (x108..1056 = 948 wide)
│   ├─ Hero savings card   x108 y84  948x150
│   │     left: "You can free up" / 48.6 GB / stacked bar + legend
│   │     divider 1x110 at x608 (card-local)
│   │     right: two drive bars (C:, D:) + legend
│   ├─ Tabs row            y260 (All 18 / Clean 11 / Optimise 7 / Review only 3), sort dropdown right (846,254 210x28); rule 948x1 at y286
│   ├─ Card grid           y304, 3 columns x 3 rows
│   └─ Link "Show 9 smaller opportunities · 1.9 GB"  y782
├─ Right panel "Your plan" x1080 y64  360 x 792 (left border 1px #D1D5DB)
│   ├─ Heading + "4 steps" count
│   ├─ 4 plan step rows (320x62, gap 8)
│   ├─ divider, Estimated total
│   ├─ "Before you apply" checklist (4 items)
│   └─ bottom actions: Estimate (dry run), Apply plan, Save as weekly schedule
└─ Status strip        y856  1440 x 44 (bg #F3F4F6, top border 1px #D1D5DB)
```

### B2 Step review + lossy confirmation dialog
```
Frame 1440x900 (same rail, strip; Clean nav item active)
├─ Header 1356x64: breadcrumb "‹ Overview" / "Meeting recordings" (18/700) + badges IN PLAN, LOSSY · OPT-IN
├─ Center (bg #FFFFFF) 996x792
│   ├─ Summary line y84 (15/600) + sub-line y108 (12/400 grey)
│   ├─ Sub-tabs y144: All 23 / Same meeting twice 6 / Need transcript 4 / Skipped 2 ; rule y170
│   └─ Table x108 y184, 848 wide: header row 30h, then 38px rows, 1px #E5E7EB separators
├─ Right panel "How to shrink them" 360x792: 3 preset tiles, 4 toggle rows, waveform preview, estimate box, "Listen before / after" button
├─ Status strip (same as B1; status text "Waiting for confirmation")
└─ Overlay: scrim 1440x900 #111827 @ 45%; modal 580x520 at (430,190) i.e. centred horizontally, top 190
```

## 2. Layout grid and measurements (px)

| Item | Value |
|---|---|
| Frame | 1440 x 900 |
| Nav rail | 84 wide, right border 1px #D1D5DB, bg #F3F4F6 |
| Rail logo tile | 26x26, r7, #111827, at (29,18); white chevron glyph 12x12, 2px stroke round |
| Rail item | active pill 64x58 at x10, r10, white, 1px #D1D5DB; items pitch 72 (y72, 144, 216, 288...), icon 22x22 r6 1.5px stroke at x31 (8px below pill top), label 11px centred full rail width 28px below icon top; Settings anchored bottom (icon y776, label y804 in B1) |
| Header | 64 high; title baseline block y19; chips y18 28h |
| Right panel | 360 wide; inner padding 20 (content x1100..1420 = 320) |
| Status strip | 44 high; controls vertically centred |
| Center content inset | 24 left/right (948 wide in B1/B2 table region 848) |
| Hero card | 948x150, r12, 1px #D1D5DB, white; padding 24; top y20 on page |
| Hero stacked bar | 440x12, r6, track #E5E7EB (segments 374/46/20 of 440 = 85%/10%/5%) |
| Drive bars | 400x12, r6, track #E5E7EB; used #94A3B8, freed #2563EB (min visible width 5) |
| Legend swatch | 8x8 r2, label 11px, 12px gap swatch to text |
| Tabs | text 13px; active 600 #111827 with 2px #2563EB underline (width = text width + ~10) at y+24; inactive 500 #6B7280; gap between tabs ~34 (A: x108,178,262,361) |
| Sort dropdown | 210x28, r6, 1px #D1D5DB; label "Sort" 12/400 grey, value 12/400 dark, chevron 8x4 1.5px stroke |
| Card grid | 3 cols x 305, column gap 16; row gap 12; card 305x146; first card at (24 from center left, 304 from frame top) |
| Card | r10, bg white; default border 1px #D1D5DB; in plan border 2px #2563EB |
| Card internals | icon tile 32x32 r8 at (16,16) (default fill #F3F4F6, in plan #DBEAFE); title 14/600 at x56,y15; subtitle 11/400 #6B7280 at y34 (single line, ellipsised ~229px); size 24/700 at (16,62); unit/count 12/400 grey at x+~14 right of size, baseline aligned (y72); badge row at y112; button row y106 28h; IN PLAN badge top right at (209,18) 54x18 |
| Card buttons | Review (text link, 64x28) at x142 + "+ Add" (outlined 80x28) at x210 (right edge 16 from card). In-plan card: single outlined "Review" 76x28 at x213 |
| Badge | 18 high, r9 (pill), 10/600 uppercase, text centred, horizontal padding ~10 (widths: 54 / 72 / 78 / 96) |
| Plan step row | 320x62, r8, bg #F9FAFB, 1px #E5E7EB, gap 8; drag grip 8x8 at (10,22) stroke #9CA3AF 1.2; title 13/600 at x28,y12; sub 11/400 grey at y33 (210 wide, ellipsised); size 13/700 #15803D right-aligned at x230 (60w); remove "×" 14/400 #9CA3AF at x298 |
| Plan estimated total | rule 320x1 #E5E7EB; label 13/400 grey; value 22/700 right-aligned |
| Checklist | heading 10/600 uppercase #6B7280; icons 16x16 circles; row pitch 26; text 12/400 #374151, 10px gap to icon (x26); last item wraps 2 lines, with 14px extra gap before it |
| Plan buttons | Estimate 320x34 (outline, 12/500); Apply 320x38 (primary, 14/500 white); link "Save as weekly schedule" 12/500 centred; vertical gaps 8 and 12 |
| Status strip | text 12/400 grey, values 13/700 dark; progress track 110x6 r3 #D1D5DB at x166; vertical separators 1x24 #D1D5DB at x430 and x760; status dot 8x8 #9CA3AF; Undo button 132x28 (x1186), Last report 98x28 (x1326), text 12/500 |
| Table (B2) | 848 wide; header row 30h bg #F9FAFB, labels 11/600 #6B7280; header bottom 1px #D1D5DB; rows 38 high, 1px #E5E7EB; checkbox 14x14 r3 at x10; file icon 12x16 1.3px #9CA3AF at x44; columns x(rel table): Recording 64 (252w), Source 310, Length 402, Size 476, After 552, Notes 626 (214w); cell text 12/400 |
| Right panel (B2) | heading 16/700 at y20; preset tile 320x50 r8 pitch 58, radio 14x14 at (12,18) (inner dot 6x6); title 13/600 at (36,9); sub 11/400 at y27. Toggle rows pitch 20-30 starting y232; switch 28x16 r8 right-aligned (knob 12x12, 2px inset); number input 74x26 r6 |
| Waveform | y345..411 area, width 320, bars 2px wide, 4px pitch, r1; silence band 46px tall highlight r2; centre line dashes = 2x2 dots |
| Estimate box | 320x40 r6 #DCFCE7, padding 12; label 11/400 #15803D; value 12/600 |
| Listen button | 320x28 outline r6 12/500 |
| Modal | 580x520, r14, white, no stroke; padding 28; sections: header 28..(icon 40x40 circle); stat tiles 4x(125x58) r8 #F9FAFB gap 8 at y112 (x28); Originals row y188; callout 524x73 r6; online-only button 203x26 r6 (x42 from modal left+14 inset); checkbox row; footer rule at y448 1px #E5E7EB; footer buttons y468 34h |
| Radii summary | pills 9, chips 14, buttons 6, inputs 6, tiles 8, cards 10, hero 12, modal 14, toggles 8, checkbox 3 |
| Icon sizes | rail 22 (+logo 26), checklist 16, scope-chip folder 14x10, grip 8, radio 14, checkbox 14, modal alert 40, chevron 8x4 |

### Type styles (Inter)
| Role | Size/weight |
|---|---|
| Page title | 18/700 |
| Hero number | 40/700 |
| Card size | 24/700 |
| Plan total | 22/700 |
| Modal title | 20/700; modal stat value 18/700 |
| Panel heading | 17/700 (Your plan), 16/700 (How to shrink them) |
| Step summary | 15/600 |
| Card title, tab, preset title, body 13 | 13-14/600 or 500 |
| Body/UI | 12/400 or 500 |
| Small / captions / rail labels | 11/400-600 |
| Badge, section eyebrow | 10/600 uppercase |

## 3. Components and states

- **Nav item**: default icon stroke #6B7280, label 11/500 #6B7280, no pill. Active: white pill 64x58 r10 1px #D1D5DB, icon stroke #2563EB, label 11/600 #111827. B1 active = Overview, B2 active = Clean. Items: Overview, Clean, Optimise, Archive, History, Settings (bottom).
- **Scope chip**: 28 high, r14, #F3F4F6, folder icon + 12/400 path; "+ Add location" 12/500 #1D4ED8 text link.
- **Buttons**: primary (#2563EB fill, white 14/500, r6, 38h, Apply); secondary/outline (white, 1px #D1D5DB, 12/500 #111827, r6, 28h; Estimate 34h); text link (no fill, 12/500 #1D4ED8; Review on default cards, Last report, Save as weekly schedule, Compare a sample first 13/500); disabled-looking text (#9CA3AF on #F3F4F6, Undo last action); warning acting button (#F59E0B fill, text 13/500 #111827, 176x34).
- **Opportunity card**: default (1px #D1D5DB, grey icon tile, Review link + "+ Add"); in plan (2px #2563EB, #DBEAFE icon tile, IN PLAN badge, outlined Review only). The "+ Add" press moves card to in-plan and adds a plan step.
- **Badges** (bg / text): REVERSIBLE, REBUILDABLE, LIGHT COPY #DCFCE7 / #15803D; LOSSY · OPT-IN and LOSSY #FEF3C7 / #B45309; IN PLAN #DBEAFE / #1D4ED8; row badges: "same meeting ×2", "duplicate", "screen + voice" #EDE9FE / #7C3AED (violet = group/kind info); "silence 14 min", "silence 6 min" #FEF3C7 / #B45309; "needs transcript" #F3F4F6 / #6B7280; "confidential" #F3F4F6 / #374151.
- **Tabs**: see section 2; the count follows the label with one space.
- **Sort dropdown**: closed state only drawn.
- **Plan step row**: grip, title, optional LOSSY badge (42x18, after title at x246), sub line, size in success green, "×" remove.
- **Checklist icons**: green check (#15803D filled circle, white tick 7x6 1.6 stroke), amber "!" (#F59E0B circle, white 11/700), blue "!" (#2563EB circle, white).
- **Drive bar**: track #E5E7EB, used #94A3B8, freed #2563EB; row label 12/600, right text 12/400 grey.
- **Stacked savings bar**: green #22C55E reversible, amber #F59E0B lossy, grey #9CA3AF review only.
- **Table row**: default (white, checked box #2563EB filled with white tick 8x6); selected (bg #DBEAFE + 3px #2563EB left bar, 38h; "Steering committee 14 Oct.wav"); unchecked/confidential (empty box white, 1px #D1D5DB; After shows "—"; confidential badge).
- **Preset tile**: selected (bg #DBEAFE, 1px #2563EB, radio filled #2563EB ring); unselected (white, 1px #D1D5DB, empty ring #D1D5DB).
- **Toggle**: on #2563EB with knob on right; off #D1D5DB with knob at left.
- **Number input**: 74x26, r6, 1px #D1D5DB, value 12/400, suffix "s max" 11/400 grey.
- **Waveform preview**: bars #6366F1 (indigo), silent stretches rendered as grey dots #9CA3AF over an amber band #FEF3C7.
- **Estimate box**: see above (green).
- **Dialog tiles**: #F9FAFB r8; label 11 grey; value 18/700 (Saving value #15803D).
- **Warning callout**: #FEF3C7, 3px left bar #B45309, title 12/600 #B45309, body 11/400 #374151.
- **Checkbox**: 14x14 r3; checked #2563EB with white tick; unchecked white with 1.2px #D1D5DB.
- **Dropdown (modal)**: 412x32, r6, 1px #D1D5DB, 12/400, chevron right.

## 4. Colours (hex, role)

| Hex | Role |
|---|---|
| #2563EB | accent: primary button, active tab underline, in-plan border, selected row bar, toggle on, checkbox, freed-by-plan bar |
| #1D4ED8 | accent text: links, IN PLAN text |
| #DBEAFE | accent tint: IN PLAN badge, in-plan icon tile, selected preset, selected row |
| #22C55E | success (bar segment); #15803D success text/icons, REVERSIBLE text, saved-size values; #DCFCE7 success tint |
| #F59E0B | warning: lossy bar segment, warning button, amber check icon; #B45309 warning text/callout bar; #FEF3C7 warning tint |
| #7C3AED / #EDE9FE | info/grouping badges (violet) |
| #6366F1 | waveform bars (indigo) |
| (none) danger | no red is used in Option B |
| #111827 | primary text, logo tile, modal scrim (45%) |
| #374151 | secondary text |
| #6B7280 | muted text, inactive nav/tab |
| #9CA3AF | placeholder, grip, "×", disabled, idle dot, review-only bar segment |
| #94A3B8 | used-space bar (slate) |
| #D1D5DB | borders, dividers, off toggle |
| #E5E7EB | hairlines, bar tracks, row separators |
| #F3F4F6 | rail, strip, chips, grey icon tile |
| #F9FAFB | center bg (B1), table header, plan step bg, tiles |
| #FFFFFF | surfaces |

## 5. On-screen copy (verbatim)

### B1 Overview
- Rail: Overview, Clean, Optimise, Archive, History, Settings
- Header: Overview | C:\Users\Michel | D:\Projects | + Add location | Scanned 2 min ago  ·  cached | Scan again
- Hero: You can free up | 48.6 GB | across 18 opportunities | legend: Reversible 41.2 GB, Lossy, opt-in 5.1 GB, Review only 2.3 GB
- Drives: "C:  System" 45 GB free  →  86 GB after plan; "D:  Projects" 354 GB free  →  362 GB after plan; legend: Used, Freed by your plan, Free
- Tabs: All 18 | Clean 11 | Optimise 7 | Review only 3 | Sort: Biggest saving first
- Cards (title / description / size / count / badge / buttons):
  1. Exact duplicates / 214 files in 61 groups · keeps newest / 12.4 GB / 214 files / reversible / IN PLAN, Review
  2. Transparent compression / NTFS LZX on cold folders, files stay usable / 14 GB / 9 folders / reversible / Review, + Add
  3. Developer artefacts / node_modules ×14 · .venv ×3 · target/ ×2 / 9.6 GB / 19 folders / rebuildable / IN PLAN, Review
  4. App caches & junk / Browsers, Teams, crash dumps, logs / 6.9 GB / 38 apps / reversible / IN PLAN, Review
  5. Screen recordings / Low-motion video → 10 fps AV1, text stays sharp / 6.1 GB / 31 videos / lossy · opt-in / Review, + Add
  6. Inactive engagements / 7 client folders idle 18+ months → 7z archive / 5.5 GB / 7 folders / reversible / Review, + Add
  7. Meeting recordings / Speech preset, silences trimmed / 4.8 GB / 23 files / lossy · opt-in / IN PLAN, Review
  8. Office decks & workbooks / Lossless first, light copies kept beside originals / 3.6 GB / 58 files / light copy / Review, + Add
  9. Old installers / .msi .exe .iso in Downloads, older than 6 months / 3.4 GB / 42 files / reversible / Review, + Add
- Link: Show 9 smaller opportunities  ·  1.9 GB
- Your plan: "Your plan" | 4 steps
  - Exact duplicates / Keep newest  ·  others to Recycle Bin / 8.6 GB
  - Developer artefacts / To Recycle Bin  ·  rebuild with npm / pip / 9.6 GB
  - App caches & junk / Apps recreate these  ·  quarantine 7 days / 6.9 GB
  - Meeting recordings [lossy] / Opus 24 kbps mono  ·  originals to Recycle Bin / 4.2 GB
  - Estimated total 29.3 GB
  - Before you apply: Every step can be undone from History for 30 days (green) | 1 lossy step will ask for confirmation (amber) | 14 signed or sensitivity-labelled files are skipped (green) | Synced OneDrive files are made online-only, never deleted in the cloud (blue)
  - Buttons: Estimate exactly (dry run) | Apply plan  ·  29.3 GB | Save as weekly schedule
- Strip: Reclaimable 48.6 GB [progress] Reclaimed 0 B | Plan: 4 steps  ·  29.3 GB | ● Nothing running | Undo last action | Last report

### B2 Step review
- Header: ‹  Overview  /  Meeting recordings  [IN PLAN] [LOSSY · OPT-IN]
- Summary: 23 recordings  ·  4.8 GB  →  about 0.6 GB
- Sub: Found in Teams, Zoom and local Recordings folders. 6 are the same meeting saved twice (audio fingerprint).
- Tabs: All 23 | Same meeting twice 6 | Need transcript 4 | Skipped 2
- Columns: Recording | Source | Length | Size | After | Notes
- Rows (name / source / length / size / after / badge; checked unless noted):
  1. Weekly sync – Nordbank.mp4 / Teams / 1:02:14 / 412 MB / ~31 MB / same meeting ×2
  2. Weekly sync – Nordbank (1).m4a / Zoom / 1:02:09 / 118 MB / delete / duplicate
  3. Steering committee 14 Oct.wav (selected) / Local / 1:47:30 / 1.1 GB / ~52 MB / silence 14 min
  4. Interview – CFO Acme.m4a / Local / 0:48:02 / 88 MB / ~9 MB / needs transcript
  5. Workshop day 1 – recording.mp4 / Teams / 3:12:44 / 1.4 GB / ~96 MB / screen + voice
  6. Expert call – Supply chain.mp3 / Zoom / 0:55:10 / 76 MB / ~10 MB / (none)
  7. Board session – privileged.m4a (UNCHECKED) / Local / 2:01:00 / 210 MB / — / confidential
  8. Kick-off – Project Falcon.wav / Local / 1:10:20 / 712 MB / ~34 MB / silence 6 min
  9. Daily standup 2026-09-30.mp4 / Teams / 0:16:08 / 96 MB / ~4 MB / (none)
- Right panel: How to shrink them | Speech (Opus 24 kbps mono · smallest) [selected] | Transcription-safe (Opus 48 kbps · keeps detail for speech-to-text) | Lossless (FLAC · no quality loss, ~45 % smaller) | Downmix to mono [on] | Remove long silences [on] + input "2" "s max" | Strip cover art [on] | Transcribe on this PC, then archive audio [off] + "Keeps a searchable text file next to each meeting" | waveform title "Steering committee 14 Oct.wav" + "14 min of silence" | This step: 4.8 GB  →  ~0.6 GB   (−4.2 GB) | ▶  Listen before / after
- Strip: same as B1 except "Waiting for confirmation".
- Dialog: Compress 23 recordings? | This is lossy. The new files sound fine for speech but cannot be turned back into the originals. | tiles Files 23, Now 4.8 GB, After ~0.6 GB, Saving 4.2 GB | Originals: [Move to Recycle Bin (default)] | callout "6 recordings are in OneDrive – Acme" / Removing the originals there also removes them in the cloud and may fall under your organisation’s retention policy. | [Make them online-only instead] | [x] I understand the new files are lower quality than the originals | Every change is logged; History → Restore brings the originals back for 30 days. | Compare a sample first | Back | Compress 23 files (amber)

## 6. Annotation notes (verbatim) and feature mapping

Feature IDs are defined in `docs/spec/file-cleaner-optimiser-features.md`.

**Note / How the zones map** (B1)
- TOP becomes scope chips + Scan again; mode switch is replaced by Clean / Optimise tabs (SCAN-01)
- LEFT category tree becomes a nav rail plus cards; each card is a detector or optimisation target with live size (SCAN-10)
- RIGHT inspector becomes "Your plan": ordered steps, total, safety checklist, dry run, apply (OPT-ENG-03)
- BOTTOM strip unchanged: reclaimable vs reclaimed, queue, undo, report

**Note / Why it is safe** (B1)
- Every card carries a safety badge: reversible, rebuildable, lossy opt-in, light copy
- Hero bar splits reversible from lossy savings so nobody is surprised (principle: estimate first)
- Plan saves as a scheduled profile (AUT-02)

**Note / Step review** (B2)
- Grid scoped to one step, with tabs for sub-groups such as same meeting twice (CLN-DET-05)
- Right panel offers 3 presets first, then a few toggles (OPT-AUD-01 to 08); waveform shows trimmed silences
- Confidential file auto-unchecked and labelled (SAF-07)

**Note / Confirmation dialog** (B2)
- Lossy actions always confirm, with a before / after summary (SAF-11)
- Synced-folder warning offers online-only instead of delete (SAF-10, CLN-ACT-07)
- Originals policy chosen in the dialog (OPT-ENG-06); undo window stated (SAF-03)
- Acting button uses the warning colour, not the normal blue, because the action is lossy

Header text of the option: "Scan once, then the app shows ranked "opportunities" as cards. The user adds the ones they want to a plan, reviews each step with only the settings that matter, and applies everything at once. Filters and the full grid stay one click away inside each step." Trade-off stated: "Power users need extra clicks to reach raw filters and the rule builder; two levels (overview → step) instead of one screen, which departs from the spec’s single-screen rule."

Element to ID: scope chips SCAN-01; card sizes / hero SCAN-10; Dry run + Apply OPT-ENG-03; weekly schedule AUT-02; sub-tab "Same meeting twice" CLN-DET-05; presets/toggles OPT-AUD-01..08; confidential row SAF-07; confirm dialog SAF-11; OneDrive callout SAF-10 + CLN-ACT-07; Originals dropdown OPT-ENG-06; History restore line / Undo SAF-03.

## 7. Ambiguities and truncations

- Several card descriptions and plan sub-lines are ellipsised in the render though the stored text is full (e.g. "quarantine 7 days" shown as "quarantine 7 d…", "originals to Recycle Bin" as "originals to Rec…"). Implement single-line ellipsis; keep the full text as tooltip.
- Nav rail frame is 856 tall in a 900 frame; assume full height above the status strip (856).
- Stroke weights for 1px borders are not stored; 1px assumed. Card shadow: none present.
- Table in B2 is partly hidden behind the dialog (Source, Length, Size, After, Notes columns visible only in the JSON, columns 310 to 840); the rows beyond the 9 shown (23 total) are not drawn; no pagination/scroll UI is shown.
- Row 3 selected styling is "selected row" not hover; row 6 and 9 have no Notes badge.
- Stacked bar numbers: 41.2 + 5.1 + 2.3 = 48.6 GB; plan total (8.6 + 9.6 + 6.9 + 4.2 = 29.3 GB) differs from card sizes (12.4, 9.6, 6.9, 4.8), so plan step sizes are estimates after dedupe/compression; "Reclaimable 48.6 GB" in the strip equals the hero.
- Header chip row shows two scopes; behaviour for overflow, removing a scope and the "+ Add location" picker are not drawn.
- Hero "14 GB" card uses a different unit format than "12.4 GB"; no rule given for decimals.
- Sort dropdown options, "Review only" tab semantics and the card-less "Review only 3" content are not shown.
- Checklist blue "!" item is informational (blue), distinct from the amber warning; blue is used for info, not only accent.
- The B2 modal sits over the whole frame (rail and strip dimmed); focus trap/ESC behaviour is not specified. The checkbox appears pre-checked in the wireframe; Compress should be disabled until it is checked (inferred, not drawn).
- Mixed "·" (middle dot) and double spaces around separators in copy are verbatim; "–" en dash in file names.
- Active rail item in B2 is "Clean" while the step is a plan item that also appears under Optimise on B1 (Meeting recordings); mapping of steps to rail sections is unspecified.
