<p align="center">
  <img src="public/icons/icon-192.png" alt="" width="96" height="96">
</p>

<h1 align="center">fitness-memo</h1>

<p align="center">
  An offline workout log PWA — no account, no server, no network.
</p>

<p align="center">
  <a href="https://asugawara.github.io/fitness-memo/"><b>Open the app</b></a>
  &nbsp;·&nbsp;
  <a href="README.ja.md">日本語</a>
</p>

A personal workout log you launch from the iPhone home screen and use completely offline.
It has no server and no sign-up; your log lives only in the device's `localStorage`.
It is built around one loop and nothing else: look at what you did last time, then do the same or a little more.

The app speaks **English and Japanese**. It follows your browser's language on first launch, and you can change it under Settings → Language.

## Screens

There are three tabs: **Record / Progress / Settings**. The Record tab puts a month calendar and the selected day's editor on one screen — tap a day cell and the editor below becomes that day's ([Make the Record tab a single screen: calendar plus day editor](adr/ux/record-tab-calendar-with-day-editor.md)). The Settings tab is a list of sections — **Export / Import, Routines, Exercises, Language, and how to add the app to your home screen** — and tapping one takes you into it ([Make the Settings tab a list of sections and push the contents one level down](adr/ux/settings-as-a-list-of-sections.md)).

| Record | Progress | Settings |
|---|---|---|
| ![Record tab](assets/1-record.png) | ![Progress tab](assets/2-progress.png) | ![Settings tab](assets/3-menu.png) |

Retake the screenshots with `trunk build && node scripts/shots.mjs`. The device (an iPhone 15 Pro), standalone launch, locale and the seeded records are all fixed, so after a UI change all three can be refreshed under identical conditions.

> [!NOTE]
> The screenshots currently show the Japanese UI. `scripts/shots.mjs` pins `locale: 'ja-JP'` — without it the seed data, which looks exercises up by their Japanese names, cannot be found and the run fails. Switch that line to `en-US` (and translate the seed) to shoot the English UI.

## Icons and share images

Everything is generated from one master, `assets/icon-master.png` (1024x1024). After replacing it, run both scripts:

```sh
sh scripts/gen-icons.sh   # public/icons/*.png — favicon, home screen, manifest
sh scripts/gen-og.sh      # public/og.png (1200x630) + assets/social-preview.png (1280x640)
```

`public/og.png` is the Open Graph image for links to **the app**. Only crawlers and social scrapers ever request it, so it is deliberately left out of the Service Worker's offline shell ([Hard-code crawler metadata to the production URL and keep it out of the offline shell](adr/seo/crawler-metadata-and-hardcoded-origin.md)). Its URL carries no content hash, so after releasing a new one you have to re-scrape it in Facebook's Sharing Debugger and X's Card Validator, or those platforms keep showing the old picture.

`assets/social-preview.png` is the image GitHub shows when **this repository** is linked. It is never served from the site — it lives only in the repo and on GitHub's CDN, which is why it sits in `assets/` rather than `public/`.

> [!IMPORTANT]
> **Uploading the social preview is a manual step.** GitHub exposes no API for it.
> Go to Settings → General → Social preview → Edit → *Upload an image…* and pick
> `assets/social-preview.png`. Regenerating the file changes nothing on GitHub until you re-upload it.

## What it does

- **See last time, then copy it** — every exercise shows last time's set count, reps and weight. "Copy last time" appears only while today's sets are still empty, and prefills today's fields in one tap.
- **Routines** — name a set of exercises you often do together and save it. On an empty day those routines are offered as candidates, and one tap lays out all the cards. The numbers that come in are pulled **per exercise, from whichever day that exercise was last done** — so a bench press you did outside your "chest day" still counts. Exercises you have never done show up as empty cards ([Start from a saved routine](adr/ux/start-from-a-saved-routine.md) / [Store routines as nothing but a name and a list of exercise IDs](adr/data-model/routines-as-named-exercise-lists.md)).
- **Turn a day into a routine** — two ways. Build one by picking exercises in the Settings tab, or press **"+ Save this day as a routine"** under the day in the Record tab. The second works on any day you picked in the calendar and starts with that day's exercises already selected, so all that's left is naming it. The sheet it opens is the same one the Settings tab uses, so you can add and drop exercises before saving ([Let a day's record become a routine directly](adr/ux/save-a-day-as-a-routine.md)).
- **Add records from the calendar** — the Record tab is one screen with the month grid above and the selected day's editor below. Days you trained carry dots in their muscle-group colours. Even on a day with no records, **tapping the cell makes the editor below that day's**, so a session you forgot to log yesterday goes in without switching tabs.
- **Grouped by muscle group** — 28 exercises across 6 groups (chest / back / shoulders / arms / legs / core) are seeded on first launch. The Exercises section lists **only the groups**; tapping one opens its exercises, and only one is open at a time. Names and colours are edited from the pencil on the right. Groups and exercises can be added, renamed and deleted ([Make the exercise list a collapsible list of muscle groups with one open at a time](adr/ux/menu-groups-as-single-open-accordion.md)).
- **A chart of the work you did** — per exercise or per muscle group, as a line. The metric switches on the spot between volume = Σ(weight × reps), set count, and rep count. Periods are 1M / 3M / 6M / 1Y / All.
- **Body weight on a second axis** — your recorded body weight is **always** overlaid on the same chart as a dashed line on the right axis; there is no toggle. The point is to read "the weight went up, but so did I" on one screen. The right axis is scaled to the data rather than starting at zero, so changes of a few hundred grams stay visible ([Always overlay body weight on the progress chart's second axis](adr/ux/body-weight-second-axis-always-on.md)).
- **Time since your last session** — overall and per muscle group. **Days are counted in local calendar days**: once the date rolls over you get "Yesterday / 3 days ago", and only within the same day do you get "45 min / 12 hr". Dividing elapsed time by 24 hours would roll over 24 hours after you trained, so last night's record would still read "today" the next morning ([Count elapsed days in local calendar days and keep clock granularity inside one day](adr/data-model/elapsed-in-local-calendar-days.md)).
- **Body weight and a note for the day** — one line per day. It can be recorded on days you did not train, and those days still appear on the chart.

**The metric is a property of the chart, not of the exercise.** Per-exercise units cannot be compared on one axis, and if an exercise's character changes later, every past chart breaks retroactively ([Make the metric a view setting rather than a property of the exercise](adr/data-model/metric-is-a-view-setting.md)). The weight field is shown for every exercise, and **an empty weight counts as 1**, so bodyweight and timed exercises work by simply leaving it blank.

## How it is built

| Area | Choice |
|---|---|
| Language / framework | Rust + [leptos](https://leptos.dev/) 0.8 (CSR), built with [trunk](https://trunkrs.dev/) |
| Routing | None. Tabs are an enum in a signal |
| Charts | No library — the SVG is drawn by hand |
| Icons | [lucide](https://lucide.dev/) (ISC, some MIT) SVGs kept in `assets/icons/` and embedded with `include_str!`. No npm dependency, no CDN |
| i18n | No crate. `src/i18n.rs` holds one struct and two `const` tables, so a missing string is a compile error ([Hand-roll the string table instead of adding an i18n crate](adr/architecture/i18n-hand-rolled-string-table.md)) |
| Persistence | The whole JSON under a single `localStorage` key, `fitness-memo/v3` (older `v2` / `v1` are read-only fallbacks) |
| CSS | One plain CSS file plus custom properties |
| Deploy | GitHub Pages branch deploy (`/docs` on the `release` branch) |
| CI | **No GitHub Actions workflow files.** Everything runs locally from `.githooks/pre-commit` |

The UI layer (whatever depends on `leptos` / `web-sys`) lives under `[target.'cfg(target_arch = "wasm32")'.dependencies]`, so `cargo test` never builds leptos's dependency graph for the host. Pure logic is concentrated in `src/core.rs` (calculations over `Db`), `src/chart_layout.rs` (chart geometry) and `src/i18n.rs` (the string tables) — that is what the unit tests cover.

> [!NOTE]
> `index.html` and the web manifest carry **English-only** metadata, on purpose. The static layer can only hold one language, and mixing them splits how search engines classify the page ([Keep static metadata in English and switch `<html lang>` at runtime](adr/seo/static-metadata-in-english.md)). The app itself overwrites `document.documentElement.lang` to match the UI language at runtime.

## Development

### Prerequisites

- [rustup](https://rustup.rs/) — `rust-toolchain.toml` declares stable and `wasm32-unknown-unknown`, so the target installs itself the first time you run anything inside the repo
- trunk — `brew install trunk`
- Node.js (for Playwright)

### Setup

```sh
sh scripts/setup.sh
```

This points `git config core.hooksPath` at `.githooks`, runs `npm install`, and installs Playwright's browsers (Chromium / WebKit). **Without it `pre-commit` never fires once.** Since there are no GitHub Actions workflows, `pre-commit` is the only thing standing between you and a broken commit.

### Dev server

```sh
trunk serve
```

Serves on <http://localhost:8080>. **No Service Worker is registered on port 8080** (`index.html` checks `location.port`), so you never get stuck looking at a stale cache-first build while developing. To unregister one you already have, add `?sw=off` and reload twice.

### Tests

```sh
cargo test                              # pure logic in src/core.rs and src/i18n.rs (host)
trunk build                             # produces dist/, which the E2E suite serves
npx playwright test --project=chromium  # the light E2E pass
npx playwright test                     # every project (Chromium / iPhone 15 Pro (WebKit) / Pixel 7)
```

`.githooks/pre-commit` first guards against `docs/` sneaking into `main`, then runs `cargo fmt --all -- --check` → `cargo clippy --target wasm32-unknown-unknown --all-features -- -D warnings` → `cargo test` → `trunk build` → `npx playwright test --project=chromium --project=harness`. In an emergency, `SKIP_HOOKS=1 git commit` skips it.

`playwright.config.mjs` pins `locale: 'ja-JP'` so the existing specs keep exercising the Japanese UI; `e2e/i18n.spec.mjs` switches to `en-US` for the English one.

If several people (or agents) work in parallel, `dist/` and Playwright's port 4173 are shared resources. To separate the outputs, pair `trunk build --dist <dir>` with `DIST_DIR=<same dir> npx playwright test`.

## Deploy

`main` holds only sources; `docs/` on the `release` branch holds the build output and is **what GitHub Pages serves**. Never commit `docs/` to `main` — once it is there, later merges stop on modify/delete conflicts (`pre-commit` guards this).

```sh
sh scripts/bootstrap-release.sh   # once, ever
sh scripts/release.sh             # every time after that
```

`release.sh` builds with the production path layout (`--public-url /fitness-memo/`), runs the heavy E2E pass including WebKit and the iPhone emulator, and then opens a PR against `release`. Merging that PR **with a merge commit** triggers the Pages deploy. (Squash and rebase are disabled in the repository settings — with either, `main`'s commits never become ancestors of `release` and conflicts pile up.)

> [!IMPORTANT]
> **Do not disable Actions in the repository settings.**
> Even with branch deploy selected, GitHub Pages always runs an internal workflow called `pages build and deployment`. Disabling Actions stops the deploy with `Error: Actor is not allowed to trigger Actions workflows`. "We don't use Actions" means precisely **we don't write `.github/workflows/`** — nothing more.

## Install on iPhone

> [!WARNING]
> **Add it to your home screen before you log anything.**
> On iOS a Safari tab and a standalone PWA added to the home screen do not share `localStorage`. If you log records in a Safari tab first and add the app afterwards, the PWA starts with an empty database and none of those records are visible.
> You can still recover: Export from the Safari tab, then Import in the PWA (Settings → Export / Import).
> The app also warns you. When it is not running standalone, a notice appears at the end of the Record tab, below "Add exercise". It is a button, and it opens an illustrated walkthrough. **It sits below the fold so it never gets in the way of logging**, which means you have to scroll to see it.

1. Open <https://asugawara.github.io/fitness-memo/> in **Safari** on your iPhone (these steps do not work in Chrome or any other browser)
2. Tap the share button at the bottom centre of the screen
3. **Scroll down** the list and choose "Add to Home Screen"
4. Tap "Add" at the top right
5. **Launch it from the home-screen icon**, and start logging there

Once the notice on the Record tab stops appearing, you are running standalone. If it is still there, you are still in a browser tab.

After it is added it launches in airplane mode, and records can be added and edited. The same walkthrough is readable inside the app (from the notice on the Record tab, or Settings → how to add it to your home screen).

The notice can be dismissed for good with the ✕ (recorded in `localStorage` under `fitness-memo/ui/v1`, never in `Db`). Dismissing it leaves the Settings entry in place, so the walkthrough stays readable.

## Keeping your data safe

Your log exists only in this device's `localStorage`. **Piling up copies inside the same device buys nothing — "Clear History and Website Data", or changing phones, wipes them all at once** (on iOS, localStorage and IndexedDB share a deletion unit). The only thing that survives is a file you moved off the device, so that is all this feature tries to do.

From Settings → Export / Import:

- **Export** — one tap writes a **TSV** (`fitness-memo-YYYYMMDD-HHMM.tsv`). On iPhone the share sheet opens; choose **"Save to Files" → iCloud Drive / Google Drive** and it survives changing phones. It **opens directly in Google Sheets** (one row per set). The column headers follow your UI language, and import accepts either language's headers, so files exported before you switched still load ([Write TSV headers in the UI language and accept both on import](adr/storage/tsv-header-follows-the-ui-language.md)).
- **Import** — pick a file you exported. Before applying anything it shows the counts "Now" and "After import" along with exactly what will be added, and **sets today's data aside automatically** right before applying (so Undo works immediately afterwards).
  - Import is fixed to **adding only**. Nothing you already have is deleted; only missing days and missing exercises are added ([`adr/storage/import-is-merge-only.md`](adr/storage/import-is-merge-only.md)).
  - `.json` files written by older versions still load.

## Current limitations

- Switching languages renames only the **preset** exercises and muscle groups you have never edited. Anything you renamed, and anything you added yourself, keeps the name you gave it — renaming is a legitimate thing to have done, so the app never overwrites it ([Localize preset names at display time and leave renamed ones alone](adr/ux/preset-names-follow-the-ui-language.md)). Nothing stored is ever rewritten; only what is displayed changes.
- If the stored JSON fails to parse, it is set aside as `fitness-memo/v3.bak-<epoch>` rather than overwritten, the app starts from the initial state, and a notice appears once at launch (so corrupt data is never silently replaced by the presets). **There is currently no way to get the set-aside data back from within the app** (see the addendum in [`adr/storage/quarantine-on-parse-failure.md`](adr/storage/quarantine-on-parse-failure.md)).
- The exported TSV does not carry all of `Db`. **IDs, muscle-group colours, ordering, archived state and the time of day** are dropped and rebuilt on import from names and the fixed preset IDs (colours and ordering revert to defaults).
- Editing in a spreadsheet and importing back is best effort. Line breaks, CRLF, `YYYY/M/D`, `62,5`, added or removed columns and reordered rows are all absorbed, but **a note starting with `=`, `+`, `-` or `@` becomes a formula in the sheet** and cannot be recovered.
- One day is one session, and one exercise gets one log per day. Dates are local, so a session that crosses midnight is split across two days.
- There is no automatic backup (on iOS neither the share sheet nor a download can be triggered without a user gesture). Exporting is manual.

## Design decisions

How the project ended up like this, and which alternatives were rejected, is recorded in [`adr/README.md`](adr/README.md) — around 70 ADRs, one per decision, grouped by category.

> [!NOTE]
> **The ADRs are written in Japanese.** They are the project's working notes and get updated on nearly every change, so they are deliberately kept in one language rather than maintained as a second translation that would go stale. The inline links throughout this README point straight at them.
