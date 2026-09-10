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

There are three tabs: **Record / Progress / Settings**. The Record tab puts a month calendar and the selected day's editor on one screen — tap a day cell and the editor below becomes that day's ([Make the Record tab a single screen: calendar plus day editor](adr/ux/record-tab-calendar-with-day-editor.md)). The Settings tab is a list of sections — **Export / Import, Routines, Exercises, Sessions shown, Drop sets, how to add the app to your home screen, an in-app manual, and Language** — and tapping one takes you into it ([Make the Settings tab a list of sections and push the contents one level down](adr/ux/settings-as-a-list-of-sections.md)).

| Record | Progress | Settings |
|---|---|---|
| ![Record tab](assets/1-record.png) | ![Progress tab](assets/2-progress.png) | ![Settings tab](assets/3-menu.png) |

Retake the screenshots with `trunk build && node scripts/shots.mjs`. The device (an iPhone 15 Pro), standalone launch, locale and the seeded records are all fixed, so after a UI change all three can be refreshed under identical conditions.

> [!NOTE]
> The screenshots above show the Japanese UI. That's a deliberate choice now, not a technical limit — `scripts/shots.mjs` feeds the same seed data into every language context (preset IDs don't change between languages; only the displayed name follows the UI language, via `ex_name` / `grp_name`), so no translation step is needed to shoot the English UI either. The README just settles on one representative language rather than keeping two synchronized copies of every screenshot. English screenshots do exist: the in-app manual is shot in both languages, under `public/manual/en/` (see "In-app manual" below).

## In-app manual

Settings has a new row, **How to use**, right after the home-screen walkthrough and just before Language. It opens as a section rather than a sheet — switching tabs and back does not lose your place — and lists eleven chapters as a one-open-at-a-time accordion, the same pattern the Exercises list already uses ([Make the in-app manual a settings section with one open chapter](adr/ux/manual-as-a-settings-section-with-one-open-chapter.md)): filtering on the Progress tab, reading the chart, copying last time, labels on an exercise, the four things "+ Note" opens, drop sets, starting from an empty day, collapsing by muscle group, reordering, export/import, and what's new. The Record tab also shows a small hint the first time only, pointing at the manual — it stays out of the way of the install-guide notice, so only one of the two ever shows at once.

Six of the eleven chapters carry a screenshot of the real screen, in both languages — twelve `.webp` files under `public/manual/{ja,en}/`, always shot in the **light theme** ([Serve the manual's figures as real screenshots of the app, not hand-drawn diagrams](adr/architecture/manual-figures-as-served-screenshots.md)). The other five go without: filtering shows nothing but two closed `<select>`s; labels are defined on one screen (the exercise edit sheet in Settings) and picked on another (the exercise card on Record), and no single screenshot can cover both, so the text carries it alone; reordering is a drag gesture a still image can't convey; export/import is already complete in words; and what's new opens as a sheet you read once and never see again, so a figure buys little. Drop sets does get one: the control that adds a stage is a bare icon with no text label (`arrow-down-wide-narrow`), and only a screenshot pins down "right next to the reps field" ([Show drop-set stages in a box under the main set and exclude them from progress by default](adr/ux/drop-sets-as-a-box-under-the-main-set.md), decision 1). Retaking a figure is safe any time — the clock and time zone are pinned in `scripts/shots.mjs`, so reshooting months later never moves "today", and produces byte-identical output when nothing actually changed:

```sh
node scripts/shots.mjs                     # everything: README's 3 + the manual's 12
node scripts/shots.mjs --only=manual       # just the manual's 12 (what pre-commit runs)
node scripts/shots.mjs --only=readme       # just README's 3 (run by hand once the UI settles)
node scripts/shots.mjs --only=manual:<id>  # one chapter, both languages, e.g. --only=manual:copy-last
node scripts/shots.mjs --check             # shoot to a scratch dir and byte-compare; exits 1 on drift
```

`.githooks/pre-commit` only ever calls `--only=manual`, and only for a commit that touches a UI-affecting path — README's three are a deliberately manual, occasional job, and `scripts/release.sh`'s `--check` is meant to catch a forgotten reshoot before a release ships ([Reshoot the manual's figures in pre-commit, but only for commits that touch a UI-affecting path](adr/deploy/screenshots-in-pre-commit-on-ui-paths.md)). The figures add up to about 214KiB but are never in the Service Worker's offline shell: offline, the manual says so and falls back to its text, so nobody who never opens it should pay for it on every install.

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

- **See your past sessions, then copy the last one** — every exercise shows past sessions **by date** (`Aug 20 (Wed)`, never "3 days ago": with several lines stacked you would be doing subtraction, whereas dates line the weekdays up so a weekly rhythm reads straight off the card). **How many to show is a setting — "Sessions shown", 1 to 3**, defaulting to 1. They are ordered newest first, so the top line is always last time. Notes are deliberately left out: this block is for reading numbers down a column to see whether you are progressing or stalling, and free text breaks the alignment ([Show past records by date and make the count a view setting](adr/ux/past-records-by-date-with-a-count-setting.md)). "Copy last time" appears only while today's sets are still empty, and prefills today's fields in one tap — **always from the most recent session only, however many are on screen**.
- **Machine pins** — save the pin positions of a machine (seat height, bar position, backrest angle) per exercise, as many as you need. Unlike set and exercise notes these **stick to the exercise and outlive the day**, so the next time you sit down at that machine you can reproduce the setup. They show as small dim text under the sets, so reading them takes no taps; you only open them from "+ Note" to change them ([Keep machine pins on the exercise and share the note toggle](adr/ux/machine-pins-on-the-exercise.md)).
- **Labels, and a "last time" per label** — if you vary your aim on the same exercise (heavy triples one day, sets of ten the next), the plain "last time" is useless: it shows whichever session came last, and the numbers are nothing alike. Define your own labels per exercise under Settings → Exercises, and a row of chips appears on that exercise's card. Tap one and **both the past sessions and what "Copy last time" fills in switch to that label**, so what you read is always what you would copy. The first chip is **"Any"** — the original behaviour, always visible, one tap away. A label with no history says "No records" and hides the copy button rather than quietly falling back to a different aim. Labels are referenced by ID, so **renaming one keeps months of history attached**. Exercises with no labels defined show no chips at all, so nothing moves for anyone who does not use this ([Pick a label with a chip and both the history and the copy switch](adr/ux/label-chips-switch-the-history-and-the-copy.md) / [Keep label definitions on the exercise and a single ID mark on the log](adr/data-model/labels-on-the-exercise-and-a-mark-on-the-log.md)). **The same chips filter the Progress tab too** — see the next item.
- **Label colours, and filtering the Progress tab by label** — give each label a colour (Settings → Exercises → the swatch on the label row). New labels get **colours that never repeat each other**, and blues are kept out of the palette so they cannot be confused with the line or the body-weight dashes. On the Progress tab, picking an exercise brings up a row of chips for its labels. Tap one and **the graph's points and the records table both narrow to that label at once**. Back on "All labels" everything shows, with **labelled days drawn in their label's colour and unlabelled days in the default one** — the chip row is the legend. The line, the axes and the body-weight dashes never change colour, so nothing is lost if you cannot read the colours. The filter is a way of looking, like the metric and the period, so it is not persisted and it resets to "All labels" when you switch exercise ([Give labels a colour and paint the Progress tab's data points with it](adr/ux/label-colour-on-the-progress-dots.md)).
- **Rest interval** — save the rest between sets, in seconds, per exercise. Exactly like pins it **sticks to the exercise and outlives the day**, so you never have to remember how long you rested last time. It shows as dim text right below the pins, and the field takes the number pad alone ([Keep the rest interval on the exercise as a single integer of seconds, right below the pins](adr/ux/interval-seconds-on-the-exercise.md)).
- **Routines** — name a set of exercises you often do together and save it. On an empty day those routines are offered as candidates, and one tap lays out all the cards. The numbers that come in are pulled **per exercise, from whichever day that exercise was last done** — so a bench press you did outside your "chest day" still counts. Exercises you have never done show up as empty cards ([Start from a saved routine](adr/ux/start-from-a-saved-routine.md) / [Store routines as nothing but a name and a list of exercise IDs](adr/data-model/routines-as-named-exercise-lists.md)).
- **Turn a day into a routine** — two ways. Build one by picking exercises in the Settings tab, or press **"+ Save this day as a routine"** under the day in the Record tab. The second works on any day you picked in the calendar and starts with that day's exercises already selected, so all that's left is naming it. The sheet it opens is the same one the Settings tab uses, so you can add and drop exercises before saving ([Let a day's record become a routine directly](adr/ux/save-a-day-as-a-routine.md)).
- **Add records from the calendar** — the Record tab is one screen with the month grid above and the selected day's editor below. Days you trained carry dots in their muscle-group colours. Even on a day with no records, **tapping the cell makes the editor below that day's**, so a session you forgot to log yesterday goes in without switching tabs.
- **Grouped by muscle group** — 28 exercises across 6 groups (chest / back / shoulders / arms / legs / core) are seeded on first launch. The Exercises section lists **only the groups**; tapping one opens its exercises, and only one is open at a time. Names and colours are edited from the pencil on the right. Groups and exercises can be added, renamed and deleted ([Make the exercise list a collapsible list of muscle groups with one open at a time](adr/ux/menu-groups-as-single-open-accordion.md)). **The record tab's add-exercise sheet uses the same accordion**: tapping a muscle group reveals its exercises, only one is open at a time, and every time the sheet opens it starts fully collapsed, so the six group rows always sit in the same place ([Make the record tab's add-exercise sheet a muscle-group accordion with one open at a time](adr/ux/record-add-sheet-groups-as-single-open-accordion.md)).
- **A chart of the work you did** — per exercise or per muscle group, as a line. The metric switches on the spot between volume = Σ(weight × reps), set count, and rep count. Periods are 1M / 3M / 6M / 1Y / All. The target is picked with **two selects, muscle group and exercise**: choosing a group narrows the exercise list to that group, and choosing an exercise fills the group in for you. Pick a group alone and you get one line summing every exercise in it. Only exercises **that have records** are listed (archived ones still show, at the end). **The last pair is remembered on the device**, so neither switching tabs nor reopening the app makes you pick again ([Make the progress target two selects: muscle group plus exercise](adr/ux/progress-target-as-group-plus-exercise-selects.md)).
- **Body weight on a second axis** — your recorded body weight is **always** overlaid on the same chart as a dashed line on the right axis; there is no toggle. The point is to read "the weight went up, but so did I" on one screen. The right axis is scaled to the data rather than starting at zero, so changes of a few hundred grams stay visible ([Always overlay body weight on the progress chart's second axis](adr/ux/body-weight-second-axis-always-on.md)).
- **Time since your last session** — overall and per muscle group. **Days are counted in local calendar days**: once the date rolls over you get "Yesterday / 3 days ago", and only within the same day do you get "45 min / 12 hr". Dividing elapsed time by 24 hours would roll over 24 hours after you trained, so last night's record would still read "today" the next morning ([Count elapsed days in local calendar days and keep clock granularity inside one day](adr/data-model/elapsed-in-local-calendar-days.md)).
- **Body weight and a note for the day** — one line per day. It can be recorded on days you did not train, and those days still appear on the chart.
- **Drop sets** — record up to four stages under a main set, for the reps you kept going for after dropping the weight. **Whether they count towards the Progress tab is a setting, and by default they do not** — mixing the dropped stages into volume inflates the baseline you use to decide "same or a little more" from one session to the next. Excluded, progress is based on your main sets alone; the Record tab's daily totals always count the stages too. **Set how much you drop by, as a percentage, and the first stage's weight is worked out for you** the moment you add it (20% by default, free input to one decimal place). From the second stage on the previous stage's weight is copied, so you only retype when you actually go lower. **Add one without opening anything** — the `↓` on the set row does it in place ([Show drop-set stages in a box under the main set and exclude them from progress by default](adr/ux/drop-sets-as-a-box-under-the-main-set.md)).
- **What's new banner** — a thin banner at the top of every tab announces new features after each release. Tap it to read every release you have not seen yet in one sheet, newest first; close it (or dismiss it with ✕) and it is gone for good — there is no menu to bring it back ([Put the what's-new banner above the screen](adr/ux/whats-new-banner-above-the-screen.md)).

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

`.githooks/pre-commit` first guards against `docs/` sneaking into `main`, then runs `cargo fmt --all -- --check` → `cargo clippy --target wasm32-unknown-unknown --all-features -- -D warnings` → `cargo test` → `trunk build` → (on a commit that touches a UI-affecting path) `node scripts/shots.mjs --only=manual` → `npx playwright test --project=chromium --project=harness`. In an emergency, `SKIP_HOOKS=1 git commit` skips the whole hook; `SHOTS=0 git commit` is narrower and skips only the reshoot, leaving fmt / clippy / test / build / E2E running — use it when the screenshots themselves are broken or you want to commit the code and the figures separately.

While iterating on one manual chapter's figure, `node scripts/shots.mjs --only=manual:<id>` (e.g. `--only=manual:copy-last`) reshoots just that chapter, both languages, instead of walking all ten.

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
- Machine pins cannot be reordered (delete with ✕ and add again with ＋), and there are at most 8 per exercise. The field opens the number pad, so a machine whose holes are labelled `A` or `red` can only get those values in from an exported file.
- The rest interval is one integer of seconds per exercise, up to 999. **There is no countdown** — it is a value you write down and read next time; timing it is your stopwatch's job. You cannot type `3:00`, and it is displayed as `180s`.
- Labels are defined per exercise and cannot be shared across exercises, at most 6 each and 12 characters long, in creation order (no reordering). Deleting one is permanent: **making a label with the same name again will not bring the old sessions back to it**, because the reference is by ID. The records themselves are never touched — they stay visible under "Any". Labels you have defined but never used are not written to the TSV (they are in the JSON backup, and typing them again restores them completely). **Label colours are not written to the TSV either** (the same as muscle-group colours; they are reassigned on import). **Above 40 points the Progress graph draws only the latest one**, so colours cannot be read over that range — filtering to a single label with a chip brings the point count down and the colours back.

## Design decisions

How the project ended up like this, and which alternatives were rejected, is recorded in [`adr/README.md`](adr/README.md) — around 90 ADRs, one per decision, grouped by category.

> [!NOTE]
> **The ADRs are written in Japanese.** They are the project's working notes and get updated on nearly every change, so they are deliberately kept in one language rather than maintained as a second translation that would go stale. The inline links throughout this README point straight at them.
