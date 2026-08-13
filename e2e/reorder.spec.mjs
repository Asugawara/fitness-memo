import { test, expect } from '@playwright/test';

// 記録タブのドラッグ並び替え（adr/ux/drag-to-reorder-in-record-tab.md）の E2E。
//
// 見るのは**永続化とデータ整合性**である。「指を dy 動かしたらどこへ落ちるか」の幾何は
// src/reorder.rs がホストの `cargo test` で総当りしているので、ここでは追わない
// （chart_layout と views::chart の分担と同じ）。
//
// ★ Playwright では再現しないものがある: `touch-action: none` の効き、長押しの
//   コールアウト、慣性スクロール、ソフトキーボード。ここが緑でも実機確認の代わりには
//   ならない。逆に「並びが Db にどう落ちるか」はここでしか見られない。
//
// ★ 掴む操作は `page.mouse` で出す。Chromium / WebKit とも本物の PointerEvent
//   （pointerType: "mouse", isPrimary: true, button: 0）になり、setPointerCapture が
//   実際に働く。`dispatchEvent` で合成した PointerEvent では capture が
//   NotFoundError で失敗し、実装が意図どおり「掴まない」ので検証にならない。

const STORAGE_KEY = 'fitness-memo/v3';

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ ──────────────────────────────────────────────────────────────────

function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

function dateKey(d) {
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${m}-${day}`;
}

/** views/mod.rs の `fmt_date` と同じ "8/8 (金)" */
function fmtDate(d) {
  return `${d.getMonth() + 1}/${d.getDate()} (${['日', '月', '火', '水', '木', '金', '土'][d.getDay()]})`;
}

/** views/calendar.rs の `fmt_month` と同じ "2026年8月" */
function fmtMonth(d) {
  return `${d.getFullYear()}年${d.getMonth() + 1}月`;
}

function daysAgo(n) {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d;
}

async function blurActive(page) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  });
}

/** hidden への visibilitychange で pending の debounce 保存を即時 flush する。 */
async function flushToStorage(page) {
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

async function readDb(page) {
  const raw = await page.evaluate((key) => localStorage.getItem(key), STORAGE_KEY);
  expect(raw, 'localStorage に Db が保存されていること').not.toBeNull();
  return JSON.parse(raw);
}

/** その日のログを種目名の列にして返す。**保存された並びそのもの。** */
async function savedOrder(page, date = new Date()) {
  const db = await readDb(page);
  const byId = new Map(db.exercises.map((e) => [e.id, e.name]));
  const session = db.sessions[dateKey(date)];
  return (session?.logs ?? []).map((l) => byId.get(l.exercise_id) ?? l.exercise_id);
}

/** その日のその種目のセットを [[重量, 回数, メモ], ...] で返す。 */
async function savedSets(page, name, date = new Date()) {
  const db = await readDb(page);
  const id = db.exercises.find((e) => e.name === name)?.id;
  const log = (db.sessions[dateKey(date)]?.logs ?? []).find((l) => l.exercise_id === id);
  return (log?.sets ?? []).map((s) => [s.weight, s.reps, s.note ?? '']);
}

async function addExercise(page, name) {
  await blurActive(page);
  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: exactText(name) })
    .click();
  return cardOf(page, name);
}

function cardOf(page, name) {
  return page
    .getByTestId('exercise-card')
    .filter({ has: page.getByTestId('card-name').filter({ hasText: exactText(name) }) });
}

async function fillSet(card, index, { weight, reps }) {
  const row = card.getByTestId('set-row').nth(index);
  if (weight !== undefined) await row.getByTestId('set-weight').fill(String(weight));
  if (reps !== undefined) await row.getByTestId('set-reps').fill(String(reps));
}

/**
 * 画面に並んでいる種目カードの名前を確かめる。
 *
 * ★ `allTextContents()` で読み比べない。reload 直後は wasm の描画前で空配列が返り、
 *   「並びが消えた」ように見えて落ちる。`toHaveText` の配列形なら描画を待ってくれる。
 */
function expectCards(page, names) {
  return expect(page.getByTestId('card-name')).toHaveText(names);
}

/**
 * カードのセット行の重量欄の値を確かめる。
 *
 * ★ `toHaveValues` は `<select multiple>` 用なので使えない。また `value` 属性は
 *   leptos が初期値として書いたきり更新されない（打鍵はプロパティ側だけを動かす）ので、
 *   属性ではなくプロパティを読む。
 */
function expectWeights(card, values) {
  return expect
    .poll(() => card.getByTestId('set-weight').evaluateAll((els) => els.map((e) => e.value)))
    .toEqual(values);
}

/**
 * ハンドルを掴んで dy だけ動かして離す。
 *
 * ★ 固定の待ち時間を入れない。`data-drag="lift"` が付くのを待てば、カードの長押し
 *   （PRESS_DELAY_CARD = 250ms）が効いたことまで一緒に確かめられる。待っている間は
 *   指が 1px も動かないので、slop（10px）で捨てられることもない。
 */
async function dragBy(page, handle, lifted, dy) {
  // ★ 測る前に画面へ入れる。boundingBox() はスクロールしないので、画面外の要素の
  //   座標をそのまま mouse へ渡すと**別の要素を掴む**（タブを往復するとスクロールが
  //   戻るので、往復後のカードは高確率で画面外にいる）
  await handle.scrollIntoViewIfNeeded();
  const box = await handle.boundingBox();
  expect(box, 'ハンドルが画面に出ていること').not.toBeNull();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;

  await page.mouse.move(x, y);
  await page.mouse.down();
  await expect(lifted).toHaveAttribute('data-drag', 'lift');
  await page.mouse.move(x, y + dy, { steps: 8 });
  await page.mouse.up();
  await expect(lifted).not.toHaveAttribute('data-drag', 'lift');
}

/** カードを dy だけ動かす。 */
async function dragCard(page, name, dy) {
  const card = cardOf(page, name);
  await dragBy(page, card.getByTestId('card-handle'), card, dy);
}

/** カード内の index 行目を dy だけ動かす。 */
async function dragSet(page, card, index, dy) {
  const row = card.getByTestId('set-row').nth(index);
  await dragBy(page, row.getByTestId('set-handle'), row, dy);
}

/**
 * ドラッグの代わりのキーボード経路。`el` にフォーカスして 1 つ動かす。
 *
 * ★ **フォーカスが載ったことを確かめてから押す。** `views::Sheet` は `<dialog>` の
 *   `close` で「開いたボタン」へフォーカスを戻す（views/mod.rs の `opener`）。この
 *   `close` イベントはブラウザが非同期に投げるので、種目を追加した直後の `focus()` の
 *   **後から**着弾しうる。そうなるとキーがカードの外（`add-exercise`）へ飛んで
 *   **何も動かない** — 負荷の高い全 project 実行でだけ落ちる形になり、実際に
 *   リリースの重い E2E で 1 度踏んだ。`toPass` で載るまで取り直す。
 */
async function nudge(page, el, up) {
  await expect(async () => {
    await el.focus();
    await expect(el).toBeFocused({ timeout: 1000 });
  }).toPass({ timeout: 5000 });
  await page.keyboard.press(up ? 'Alt+ArrowUp' : 'Alt+ArrowDown');
}

async function openDay(page, date) {
  await blurActive(page);
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  const title = await page.getByTestId('cal-title').textContent();
  const [, y, m] = title.match(/^(\d+)年(\d+)月$/);
  const delta = date.getFullYear() * 12 + date.getMonth() - (Number(y) * 12 + (Number(m) - 1));
  for (let i = 0; i < Math.abs(delta); i++) {
    await page.getByTestId(delta < 0 ? 'cal-prev' : 'cal-next').click();
  }
  await expect(page.getByTestId('cal-title')).toHaveText(fmtMonth(date));
  await page.locator(`[data-testid="cal-day"][data-date="${dateKey(date)}"]`).click();
  await expect(page.getByTestId('today-date')).toHaveText(fmtDate(date));
}

/** 3 セット入りのベンチプレスのカードを作る。 */
async function benchWithThreeSets(page) {
  const card = await addExercise(page, 'ベンチプレス');
  await fillSet(card, 0, { weight: 60, reps: 10 });
  await card.getByTestId('add-set').click();
  await fillSet(card, 1, { weight: 62.5, reps: 8 });
  await card.getByTestId('add-set').click();
  await fillSet(card, 2, { weight: 65, reps: 6 });
  await blurActive(page);
  return card;
}

// ── セット行 ────────────────────────────────────────────────────────────────

test('セット番号を掴んで動かすと並びが変わり、番号が振り直され、リロードしても残る', async ({
  page,
}) => {
  const card = await benchWithThreeSets(page);

  // 3 本目を先頭へ。1 本目の中心を越えるまで上げる
  const rowHeight = (await card.getByTestId('set-row').nth(0).boundingBox()).height;
  await dragSet(page, card, 2, -rowHeight * 2);

  await expectWeights(card, ['65', '60', '62.5']);
  // 番号は位置から毎回計算しているので、動かせば必ず振り直される
  await expect(card.getByTestId('set-handle')).toHaveText(['1', '2', '3']);

  await flushToStorage(page);
  expect(await savedSets(page, 'ベンチプレス')).toEqual([
    [65, 6, ''],
    [60, 10, ''],
    [62.5, 8, ''],
  ]);

  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expectWeights(cardOf(page, 'ベンチプレス'), ['65', '60', '62.5']);
});

test('★ ドラッグ中も番号は上から 1,2,3 で、掴んだ行は落ちる先の番号を先に示す', async ({
  page,
}) => {
  // ★ ドラッグ中は Vec を入れ替えないので、模型の添字をそのまま描くと「2 番目に
  //   見えている行に 1 と書いてある」状態になる。番号をハンドルにした理由
  //   （番号 ＝ 順番）が指を離すまで嘘になるので、見えている位置を描く
  const card = await benchWithThreeSets(page);
  const rows = card.getByTestId('set-row');
  const tops = () => rows.evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().top)));
  const before = await tops();

  await card.getByTestId('set-handle').nth(0).scrollIntoViewIfNeeded();
  const box = await card.getByTestId('set-handle').nth(0).boundingBox();
  const rowHeight = (await rows.nth(0).boundingBox()).height;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + rowHeight, { steps: 6 });

  // ★ DOM の並びはドラッグ中も変わらない（transform で見せているだけ）。掴んだ行は
  //   DOM の 1 番目のまま「2」を表示し、押しのけられた行が「1」になる。
  //   ＝ 画面では上から 1, 2, 3 に見えている
  await expect(card.getByTestId('set-handle')).toHaveText(['2', '1', '3']);
  const grabbed = rows.nth(0);
  await expect(grabbed).toHaveAttribute('data-drag', 'lift');
  await expect(grabbed.getByTestId('set-handle')).toHaveText('2');

  // ★ 見えている位置そのものを確かめる。1 本目と 2 本目が入れ替わった位置に居ること。
  //   掴んだ行はトランジション無しで即座に付いてくるが、押しのけられる側は 120ms
  //   かけて動くので poll で落ち着くのを待つ。
  //   ★ ここが `before` のままだと、CSS の詳細度で `transition: none` が負けて
  //     掴んだ行が指に遅れて付いてくる状態を見逃す
  await expect.poll(tops).toEqual([before[1], before[0], before[2]]);

  await page.mouse.up();
  await expectWeights(card, ['62.5', '60', '65']);
  await expect(card.getByTestId('set-handle')).toHaveText(['1', '2', '3']);
});

test('★ 押しのけられる行にトランジションが効いていて、掴んだ行には効いていない', async ({
  page,
}) => {
  // ★ CSS の構文エラーでルールごと捨てられていても、並びは正しくなるので他の
  //   テストは全部通る。**computed style を直接見るのがこの退行を捕まえる唯一の方法**
  //   （実際にコメントの閉じ忘れで丸ごと無効になっていた）
  const card = await benchWithThreeSets(page);
  const rows = card.getByTestId('set-row');
  await card.getByTestId('set-handle').nth(0).scrollIntoViewIfNeeded();
  const box = await card.getByTestId('set-handle').nth(0).boundingBox();
  const rowHeight = (await rows.nth(0).boundingBox()).height;

  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + rowHeight, { steps: 4 });

  const transitions = await rows.evaluateAll((els) =>
    els.map((e) => getComputedStyle(e).transitionProperty),
  );
  expect(transitions[0], '掴んだ行は指に遅れず付いてくる').toBe('all');
  expect(transitions[1], '押しのけられる行は滑る').toBe('transform');
  expect(
    await rows.nth(1).evaluate((e) => getComputedStyle(e).transitionDuration),
  ).toBe('0.12s');

  await page.mouse.up();
  // 静止時はどちらにも効いていない（親の data-dragging が外れる）
  await expect
    .poll(() => rows.evaluateAll((els) => els.map((e) => getComputedStyle(e).transitionProperty)))
    .toEqual(['all', 'all', 'all']);
});

test('セットメモは行について回る', async ({ page }) => {
  const card = await benchWithThreeSets(page);
  await card.getByTestId('note-toggle').click();
  await card.getByTestId('set-note').nth(0).fill('1本目キツい');
  await card.getByTestId('set-note').nth(2).fill('3本目つぶれた');
  await blurActive(page);

  const rowHeight = (await card.getByTestId('set-row').nth(0).boundingBox()).height;
  await dragSet(page, card, 2, -rowHeight * 2);

  await flushToStorage(page);
  expect(await savedSets(page, 'ベンチプレス')).toEqual([
    [65, 6, '3本目つぶれた'],
    [60, 10, '1本目キツい'],
    [62.5, 8, ''],
  ]);
});

test('セット番号をタップしただけでは並びも記録も変わらない', async ({ page }) => {
  const card = await benchWithThreeSets(page);
  await flushToStorage(page);
  const before = await savedSets(page, 'ベンチプレス');

  await card.getByTestId('set-handle').nth(2).click();

  await expectWeights(card, ['60', '62.5', '65']);
  await expect(card.getByTestId('set-row')).toHaveCount(3);
  await flushToStorage(page);
  expect(await savedSets(page, 'ベンチプレス')).toEqual(before);
});

test('ドラッグしてもセットは消えず、削除の ✕ も誤爆しない', async ({ page }) => {
  const card = await benchWithThreeSets(page);
  const rowHeight = (await card.getByTestId('set-row').nth(0).boundingBox()).height;

  await dragSet(page, card, 0, rowHeight * 2);

  await expect(card.getByTestId('set-row')).toHaveCount(3);
  await expectWeights(card, ['62.5', '65', '60']);
});

test('Alt + ↑↓ でもセット行が動く（ドラッグの代わりの経路）', async ({ page }) => {
  const card = await benchWithThreeSets(page);

  // 入力欄にフォーカスしたまま動かせる。★ カード側の同じハンドラには届かない
  await nudge(page, card.getByTestId('set-reps').nth(2), true);

  await expectWeights(card, ['60', '65', '62.5']);
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);

  await blurActive(page);
  await flushToStorage(page);
  expect(await savedSets(page, 'ベンチプレス')).toEqual([
    [60, 10, ''],
    [65, 6, ''],
    [62.5, 8, ''],
  ]);
});

test('★ セットメモ欄の中でも Alt + ↑↓ は行を動かす（カードは動かない）', async ({ page }) => {
  // ★ 行の入力欄のどれか 1 つでも on:keydown を漏らすと、そこからカード側へ
  //   bubble して「行を動かしたつもりがカードごと動く」。ドラッグを使えない人に
  //   とっては唯一の経路なので、入力欄ごとに見る
  const bench = await benchWithThreeSets(page);
  const push = await addExercise(page, 'プッシュアップ');
  await fillSet(push, 0, { reps: 20 });
  await blurActive(page);
  await bench.getByTestId('note-toggle').click();

  await nudge(page, bench.getByTestId('set-note').nth(2), true);

  await expectWeights(bench, ['60', '65', '62.5']);
  await expectCards(page, ['ベンチプレス', 'プッシュアップ']);
});

test('保存されない空行を動かしても、保存済みセットの並びは壊れない', async ({ page }) => {
  // ★ 空行・メモだけの行は `parse_reps` が None を返して commit に載らない
  //   （既存仕様。行内の note-orphan がその理由を出している）。動かしても JSON は
  //   変わらないのが正しく、**空行の位置が残ることを期待してはいけない**
  const card = await addExercise(page, 'ベンチプレス');
  await fillSet(card, 0, { weight: 60, reps: 10 });
  await card.getByTestId('add-set').click();
  await card.getByTestId('add-set').click();
  await fillSet(card, 2, { reps: 8 });
  await blurActive(page);
  await expect(card.getByTestId('set-row')).toHaveCount(3);

  const rowHeight = (await card.getByTestId('set-row').nth(0).boundingBox()).height;
  await dragSet(page, card, 1, -rowHeight * 1.5);

  await flushToStorage(page);
  expect(await savedSets(page, 'ベンチプレス')).toEqual([
    [60, 10, ''],
    [60, 8, ''],
  ]);

  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expectWeights(cardOf(page, 'ベンチプレス'), ['60', '60']);
});

test('★ 過去日でセットを並び替えても at は入らない', async ({ page }) => {
  // 並べ替えは実施ではない（adr/data-model/at-optional-same-day-only.md）。
  // セット側は commit() を通るので当日なら at が更新されるが、過去日は is_today の
  // ガードで絶対に押されない
  const yesterday = daysAgo(1);
  await openDay(page, yesterday);
  await expect(page.getByTestId('past-banner')).toBeVisible();

  const card = await addExercise(page, 'ベンチプレス');
  await fillSet(card, 0, { weight: 60, reps: 10 });
  await card.getByTestId('add-set').click();
  await fillSet(card, 1, { weight: 62.5, reps: 8 });
  await blurActive(page);

  await nudge(page, card.getByTestId('set-reps').nth(1), true);
  await blurActive(page);
  await flushToStorage(page);

  expect(await savedSets(page, 'ベンチプレス', yesterday)).toEqual([
    [62.5, 8, ''],
    [60, 10, ''],
  ]);
  const db = await readDb(page);
  expect(db.sessions[dateKey(yesterday)].logs[0].at ?? null, 'at は null のまま').toBeNull();
});

// ── 種目カード ──────────────────────────────────────────────────────────────

test('見出しを長押しして動かすとカードの並びが変わり、日付を往復しても残る', async ({
  page,
}) => {
  const bench = await addExercise(page, 'ベンチプレス');
  await fillSet(bench, 0, { weight: 60, reps: 10 });
  const push = await addExercise(page, 'プッシュアップ');
  await fillSet(push, 0, { reps: 20 });
  const plank = await addExercise(page, 'プランク');
  await fillSet(plank, 0, { reps: 60 });
  await blurActive(page);
  await expectCards(page, ['ベンチプレス', 'プッシュアップ', 'プランク']);

  // 3 枚目を先頭へ。上 2 枚を通り過ぎるだけ動かす
  const box = await bench.boundingBox();
  await dragCard(page, 'プランク', -(box.height * 2 + 40));

  await expectCards(page, ['プランク', 'ベンチプレス', 'プッシュアップ']);
  await flushToStorage(page);
  expect(await savedOrder(page)).toEqual(['プランク', 'ベンチプレス', 'プッシュアップ']);

  // ★ load_cards は日付が変わったときに session.logs から引き直す。
  //   cards だけ入れ替える実装だとここで元に戻る
  await openDay(page, daysAgo(1));
  await openDay(page, new Date());
  await expectCards(page, ['プランク', 'ベンチプレス', 'プッシュアップ']);

  await page.reload();
  await expectCards(page, ['プランク', 'ベンチプレス', 'プッシュアップ']);
});

test('★ まだ回数を打っていないカードを動かしてから打っても、その位置に残る', async ({
  page,
}) => {
  // ★ `write_log` の新規枝は `logs.push` なので、並び替えを画面の側だけで持つと
  //   1 文字目の commit でそのログだけ末尾に生える。commit からも reorder_logs を
  //   呼んでいることの唯一の証明
  const bench = await addExercise(page, 'ベンチプレス');
  await fillSet(bench, 0, { weight: 60, reps: 10 });
  const push = await addExercise(page, 'プッシュアップ');
  await blurActive(page);
  await expectCards(page, ['ベンチプレス', 'プッシュアップ']);

  // まだ 1 度も commit されていないカードを先頭へ（キーボード経路で確実に）
  await nudge(page, push.getByTestId('note-toggle'), true);
  await expectCards(page, ['プッシュアップ', 'ベンチプレス']);

  // ここで初めてログが生まれる
  await fillSet(push, 0, { reps: 20 });
  await blurActive(page);
  await flushToStorage(page);
  expect(await savedOrder(page)).toEqual(['プッシュアップ', 'ベンチプレス']);

  await page.reload();
  await expectCards(page, ['プッシュアップ', 'ベンチプレス']);
});

test('見出しをタップしただけ / 素早くフリックしただけでは並びが変わらない', async ({ page }) => {
  // ★ `.card-head` は全幅の帯で touch-action: none なので、即時開始にすると
  //   スクロールのつもりのフリックが黙って並びを変える。250ms の長押しで塞いでいる
  const bench = await addExercise(page, 'ベンチプレス');
  await fillSet(bench, 0, { weight: 60, reps: 10 });
  const push = await addExercise(page, 'プッシュアップ');
  await fillSet(push, 0, { reps: 20 });
  await blurActive(page);

  const head = push.getByTestId('card-handle');
  await head.scrollIntoViewIfNeeded();
  const box = await head.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;

  // タップ
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.up();
  await expectCards(page, ['ベンチプレス', 'プッシュアップ']);

  // 長押しを待たずに振り抜く（＝スクロールのつもりのフリック）
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y - 400, { steps: 8 });
  await expect(push).not.toHaveAttribute('data-drag', 'lift');
  await page.mouse.up();

  await expectCards(page, ['ベンチプレス', 'プッシュアップ']);
  await flushToStorage(page);
  expect(await savedOrder(page)).toEqual(['ベンチプレス', 'プッシュアップ']);
});

test('★ ドラッグしてもタブバーが消えず、掴むとキーボードは引っ込む', async ({ page }) => {
  // ★ WebKit だけの経路。指の下の入力欄に**フォーカスが移ってしまい**（選択ドラッグ）、
  //   その後 <For> が DOM を move した拍子に focusout を出さずにフォーカスが消えるので、
  //   `.kb-open` が立ちっぱなしになる ＝ **タブバーが消えたまま戻らない**。
  //   Chromium では再現しないが、テストは全 project で回す（塞いだ側を守る）
  const bench = await addExercise(page, 'ベンチプレス');
  await fillSet(bench, 0, { weight: 60, reps: 10 });
  const push = await addExercise(page, 'プッシュアップ');
  await fillSet(push, 0, { reps: 20 });

  // 入力欄にフォーカスを残したまま掴む（トレ中はこれが普通）
  await push.getByTestId('set-reps').focus();
  await expect(page.getByTestId('tab-record')).toBeHidden();

  await dragCard(page, 'プッシュアップ', -((await bench.boundingBox()).height + 40));

  await expectCards(page, ['プッシュアップ', 'ベンチプレス']);
  await expect(page.getByTestId('tab-record'), 'タブバーが消えたまま戻らない').toBeVisible();
  expect(
    await page.evaluate(() => document.querySelector('.app').className),
    '掴んだらキーボード状態は解除される',
  ).not.toContain('kb-open');
});

test('★ 掴んだままタブを切り替えてもアプリが落ちない', async ({ page }) => {
  // ★ 記録タブは mod.rs の `match tab.get()` の枝なので、タブを切り替えると
  //   DayEditor ごと破棄される。生き残った rAF ループと長押しタイマーが破棄済みの
  //   signal を触ると wasm が unreachable に落ちてアプリが死ぬ（画面が白くなる）。
  //   ★ タブは dispatchEvent で押す。pointer capture 中は本物のクリックが
  //     掴んでいる要素へ吸われるので、実機の「2 本目の指でタブを押す」を再現できない
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));

  const bench = await addExercise(page, 'ベンチプレス');
  await fillSet(bench, 0, { weight: 60, reps: 10 });
  const push = await addExercise(page, 'プッシュアップ');
  await fillSet(push, 0, { reps: 20 });
  await blurActive(page);

  // (1) 長押しの待ち時間の途中で切り替える（タイマーだけが生き残る経路）
  const head = push.getByTestId('card-handle');
  await head.scrollIntoViewIfNeeded();
  let box = await head.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.getByTestId('tab-menu').dispatchEvent('click');
  await expect(page.getByTestId('screen-menu')).toBeVisible();
  await page.mouse.up();

  await page.getByTestId('tab-record').dispatchEvent('click');
  await expectCards(page, ['ベンチプレス', 'プッシュアップ']);

  // (2) 掴んだ状態で切り替える（rAF ループが生き残る経路）
  await push.getByTestId('card-handle').scrollIntoViewIfNeeded();
  box = await push.getByTestId('card-handle').boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await expect(push).toHaveAttribute('data-drag', 'lift');
  await page.getByTestId('tab-menu').dispatchEvent('click');
  await expect(page.getByTestId('screen-menu')).toBeVisible();
  await page.mouse.up();

  await page.getByTestId('tab-record').dispatchEvent('click');
  await expectCards(page, ['ベンチプレス', 'プッシュアップ']);

  // ★ 戻ってきたあともドラッグが効く（EDGE_SCROLLING が立ちっぱなしだと
  //   自動スクロールだけが二度と動かなくなる。掴めること自体はここで見る）
  const again = cardOf(page, 'プッシュアップ');
  await dragBy(page, again.getByTestId('card-handle'), again, -((await bench.boundingBox()).height + 40));
  await expectCards(page, ['プッシュアップ', 'ベンチプレス']);

  expect(errors, 'ページ内で例外が起きた').toEqual([]);
});

test('Alt + ↑↓ でもカードが動き、端では何も起きない', async ({ page }) => {
  const bench = await addExercise(page, 'ベンチプレス');
  await fillSet(bench, 0, { weight: 60, reps: 10 });
  const push = await addExercise(page, 'プッシュアップ');
  await fillSet(push, 0, { reps: 20 });
  await blurActive(page);

  await nudge(page, bench.getByTestId('note-toggle'), true);
  await expectCards(page, [
    'ベンチプレス',
    'プッシュアップ',
  ]); // 先頭は上へ行けない

  await nudge(page, bench.getByTestId('note-toggle'), false);
  await expectCards(page, ['プッシュアップ', 'ベンチプレス']);

  await blurActive(page);
  await flushToStorage(page);
  expect(await savedOrder(page)).toEqual(['プッシュアップ', 'ベンチプレス']);
});
