# W2-F decisions (frontend shell, B1 Overview, B2 Step review)

Format as in `docs/DECISIONS.md`. IDs are local to this file (`F-01`...).

## F-01 Views are plain functions over one store, built with `h()` and no HTML strings
- **Context:** no bundler, no framework, tests without a browser.
- **Choice:** `ui/js/dom.js` builds elements with `createElement`/`textContent` only and refuses `onclick`-style attributes; views are `(state, actions) => element`; regions redraw when the store keys they read change; focus is restored by `data-fk` keys. Tests use an in-house DOM (`ui/js/testing/fakedom.js`), not a browser.
- **Why:** file names are untrusted; this makes injection structurally impossible and keeps the CSP free of `unsafe-inline`.

## F-02 The controller is the only writer and only IPC caller; `api` is injected
- **Choice:** `createController({ api, store })`; views get `actions`. Tests inject a recording proxy over a mock backend, so every control is asserted by the command and payload it sends.

## F-03 Progress lives outside the state that drives controls
- **Context:** `scan://progress` and `job://progress` arrive every 50 ms; re-rendering Pause/Stop/Cancel that often swallows clicks (mousedown and mouseup land on different elements).
- **Choice:** `scanProgress` and `jobProgress` are separate store keys, patched in place (`patchScanning`, `patchStrip`); controls redraw only on state changes. A test asserts the same Pause button element survives a progress event.

## F-04 Card number is `size_before`, tooltip says what it saves
- **Context:** the wireframe shows 4.8 GB on Meeting recordings but 4.2 GB in the plan; hero and plan use savings.
- **Choice:** cards show `size_before` (what the opportunity covers), tooltip "Saves about X" from `saving`. Plan steps and the total show savings. Cards are ordered by `rank` ("Biggest saving first"), which the fixtures set to the wireframe order.

## F-05 Fixture numbers
- **Context:** the wireframe is internally inconsistent (tab counts 11 + 7 + 3 exceed 18; card sizes exceed 48.6 GB; plan 29.3 GB vs card sizes).
- **Choice:** the hero summary (48.6 / 41.2 / 5.1 / 2.3 GB), drives, plan (8.6 + 9.6 + 6.9 + 4.2 = 29.3 GB) and the nine card titles/sizes are verbatim. Tab counts are derived: All 18, Clean 8, Optimise 6, Archive 1, Review only 3 (IPC: category tabs exclude review-only). Meeting recordings has 24 rows, 23 selected (so "23 recordings" and the dialog read as in the wireframe); the tab reads "All 24".

## F-06 Rail highlight on the step review follows the opportunity's category
- **Context:** the wireframe highlights Clean for Meeting recordings (an optimiser); spec §7 calls the mapping unspecified.
- **Choice:** category-based (Optimise for audio). One-line change in `views/shell/rail.js` if the owner prefers the wireframe.

## F-07 One lossy/permanent dialog per step, in plan order, apply at the end
- **Choice:** "Apply plan" fetches `plan_confirmation_info` for every step whose safety is `lossy_opt_in` or `permanent`, shows one dialog each ("Step 1 of 2"), and calls `plan_apply` once with all confirmations. Lossy: warning-style acting button; permanent: danger-style, no originals dropdown, no "Compare a sample". Escape and Back cancel everything. The step review header offers Apply too (it opens the same flow).

## F-08 Paging, not virtual scrolling
- **Choice:** the table renders one page (50 rows) from `candidates_page` with Previous/Next; the table sits in the main region's single scroller. Only `.app__main`, `.app__panel`, the job queue and modals scroll (a test enforces the list).

## F-09 Move action uses the in-app folder browser
- **Choice:** "Move…" opens the browser in `move` mode and sends `{kind:"move", dest, keep_structure:false}`. Permanent delete is under "Advanced" only.

## F-10 Mock behaviour worth knowing
- `plan_apply` on an empty plan rejects with `not_implemented` (frozen `api.test.js` expects that code for the default, empty mock). A real engine would more naturally answer `invalid_input`.
- URL switches: `?mock=1&scenario=empty|fresh|results&speed=0.5`. Default `empty`.
- `window.__mj = { store, actions }` is exposed only with the mock backend, for browser tests.

## F-11 Copy
- Single module `ui/js/copy.js`. The wireframe's em dash in the unchecked After cell is an en dash (no em dashes allowed). A guard test fails on any em dash in app code and on sentence literals in views.

## Requests to the orchestrator (not done: files are frozen)
1. `api.js`: concurrent first calls each create their own mock backend (`getBackend` awaits the dynamic import before assigning), so listeners and calls end up on different instances. `main.js` works around it by awaiting `backendKind()` first and subscribing sequentially. Memoise the promise in `getBackend`.
2. `tokens.css`: no violet token exists for group/kind row badges (spec uses violet); the `--info` blue family is used.
3. `api.test.js`: the `plan_apply` `not_implemented` expectation (see F-10) may be relaxed to `invalid_input` or `confirmation_required`.
