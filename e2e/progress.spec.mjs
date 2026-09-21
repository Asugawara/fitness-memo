// 推移タブの対象セレクタ（部位 + 種目）。
//
// ここでしか見られないのは 4 つ。
//
// - **2 つの select が実際に連動すること。** 部位で種目リストが絞られ、種目を選ぶと
//   部位が埋まる。`core::Pick` のユニットテストは組の計算しか見ておらず、
//   `<option selected>` が DOM でどう当たるかは見ていない
// - **選択が localStorage に残り、`Db` に混ざらないこと**
//   （adr/storage/db-ids-in-ui-state-behind-a-fallback.md）
// - **記録が消えた種目から既定へ寄せ直す経路**が実際に走ること。ID を UI 状態に
//   置いてよい根拠がこの受け皿なので、ここが緑でないと ADR の前提が崩れる
// - **タブを往復しても選択が戻らないこと。** `Progress` はタブ切替のたびに
//   作り直されるので、コンテキストではなく保存値から読み直している
//
// 絞り込みと並べ替えの規則そのもの（記録がある種目だけ / アーカイブ済みは末尾 /
// 壊れた保存値の扱い）は `core.rs` のユニットテストが持つ。
import { expect, test } from '@playwright/test';

const STORAGE_KEY = 'fitness-memo/v3';
const UI_KEY = 'fitness-memo/ui/v1';

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ ──────────────────────────────────────────────────────────────────

function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

/** hidden への visibilitychange で pending の debounce 保存を即時 flush する。 */
async function flushToStorage(page) {
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

/** 過去のログを localStorage へ直接注入する（smoke.spec.mjs の同名ヘルパと同じ形）。 */
async function seedPastLogs(page, entries) {
  await flushToStorage(page);
  await page.evaluate(
    ({ entries, key }) => {
      const db = JSON.parse(localStorage.getItem(key));
      const dateKey = (n) => {
        const d = new Date();
        d.setDate(d.getDate() - n);
        const m = String(d.getMonth() + 1).padStart(2, '0');
        const day = String(d.getDate()).padStart(2, '0');
        return `${d.getFullYear()}-${m}-${day}`;
      };
      for (const { daysAgo, exerciseName, sets } of entries) {
        const k = dateKey(daysAgo);
        const ex = db.exercises.find((e) => e.name === exerciseName);
        if (!ex) throw new Error(`preset exercise not found: ${exerciseName}`);
        const session = db.sessions[k] ?? { logs: [], body_weight: null, note: '' };
        session.logs.push({ exercise_id: ex.id, sets, at: null });
        db.sessions[k] = session;
      }
      localStorage.setItem(key, JSON.stringify(db));
    },
    { entries, key: STORAGE_KEY },
  );
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

/** 胸 2 種目 / 背中 1 種目に記録を入れる。部位を跨いだ操作を見るための最小構成。 */
async function seedTwoGroups(page) {
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 2, exerciseName: 'プッシュアップ', sets: [{ weight: 0, reps: 20 }] },
    { daysAgo: 1, exerciseName: '懸垂', sets: [{ weight: 0, reps: 12 }] },
  ]);
}

async function openProgress(page) {
  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('screen-progress')).toBeVisible();
}

function groupSelect(page) {
  return page.getByTestId('group-select');
}

function exerciseSelect(page) {
  return page.getByTestId('exercise-select');
}

/** 種目セレクタに今出ている選択肢のラベル（先頭の「すべての種目」を除く）。 */
async function exerciseLabels(page) {
  return page.getByTestId('exercise-select').locator('optgroup option').allTextContents();
}

async function uiState(page) {
  return page.evaluate((key) => JSON.parse(localStorage.getItem(key) ?? '{}'), UI_KEY);
}

// ── 2 つの select の連動 ─────────────────────────────────────────────────────

test('1. 部位を選ぶと種目セレクタはその部位の種目だけになる', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  // 既定は記録がある先頭の種目 = 胸/ベンチプレス。既にその部位で絞られている
  expect(await exerciseLabels(page)).toEqual(['ベンチプレス', 'プッシュアップ']);

  await groupSelect(page).selectOption({ label: '背中' });
  expect(await exerciseLabels(page)).toEqual(['懸垂']);

  // 記録がある状態では、統計と記録テーブルの容器そのものが出ている
  // （5. の「記録が無いと両方消える」の対）
  await expect(page.getByTestId('stats')).toBeVisible();
  await expect(page.getByTestId('records')).toBeVisible();
});

test('2. 部位が「すべて」だと全部位の種目が並ぶ', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  await groupSelect(page).selectOption({ label: 'すべての部位' });
  // 並びは 部位の順 → 部位内の順（胸 → 背中）
  expect(await exerciseLabels(page)).toEqual(['ベンチプレス', 'プッシュアップ', '懸垂']);
});

test('3. 種目を選ぶと部位セレクタにその種目の所属部位が入る', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  await groupSelect(page).selectOption({ label: 'すべての部位' });
  await expect(groupSelect(page)).toHaveValue('');

  await exerciseSelect(page).selectOption({ label: '懸垂' });

  // ★ 部位が自動で埋まり、以降は背中で絞られる。1 手で「その部位の他の種目」へ行ける
  await expect(groupSelect(page).locator('option:checked')).toHaveText('背中');
  expect(await exerciseLabels(page)).toEqual(['懸垂']);
  // 懸垂 12 回。重量なしでも 0 に潰れない
  await expect(page.getByTestId('stat-best')).toHaveText('12');
});

test('4. 部位を切り替えると別部位の種目は外れ、その部位の合計になる', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  await groupSelect(page).selectOption({ label: '背中' });
  await exerciseSelect(page).selectOption({ label: '懸垂' });
  await expect(page.getByTestId('stat-best')).toHaveText('12');

  // 懸垂は胸に属さないので落ちる → 胸の合計（ベンチ 600 / プッシュアップ 20）
  await groupSelect(page).selectOption({ label: '胸' });
  await expect(exerciseSelect(page)).toHaveValue('');
  await expect(page.getByTestId('stat-best')).toHaveText('600');
});

test('5. 部位を「すべて」に戻すと種目も外れ、グラフの代わりに案内が出る', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);
  await expect(page.getByTestId('chart')).toBeVisible();

  // ★ 部位の「すべて」は絞り込みの解除ではなくリセット。2 つの select が
  //   「部位=すべて / 種目=懸垂」のように食い違って見える状態を作らない
  await groupSelect(page).selectOption({ label: 'すべての部位' });
  await expect(exerciseSelect(page)).toHaveValue('');
  await expect(page.getByTestId('progress-pick-hint')).toContainText('部位か種目を選んで');
  await expect(page.getByTestId('chart')).toHaveCount(0);
  // 統計も記録テーブルも一緒に消える（対象が無いのに数字だけ残さない）
  await expect(page.getByTestId('stats')).toHaveCount(0);
  await expect(page.getByTestId('records')).toHaveCount(0);
});

// ★ **一度触った `<option>` を経由する順序で回す。** content attribute の `selected` は
//   HTML の dirtiness が立った `<option>` では selectedness を動かさないので、
//   `selected=` のままだと 2 つの select が食い違う（部位=すべて / 種目=懸垂 の表示）。
//   触っていない option だけを通る手順では通ってしまうため、**先に選んでおく手順が要る**。
test('11. 一度選んだ部位を経由しても 2 つの select は食い違わない', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  // 背中の option を dirty にしてから、いったん「すべて」を経由して戻る
  await groupSelect(page).selectOption({ label: '背中' });
  await groupSelect(page).selectOption({ label: 'すべての部位' });
  await exerciseSelect(page).selectOption({ label: '懸垂' });

  await expect(groupSelect(page).locator('option:checked')).toHaveText('背中');
  await expect(exerciseSelect(page).locator('option:checked')).toHaveText('懸垂');
  expect(await exerciseLabels(page)).toEqual(['懸垂']);
});

test('12. 一度選んだ種目を経由しても統計はセレクタの表示と一致する', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  // 胸の 2 種目を両方 dirty にしてから部位を外し、別部位の種目へ飛ぶ
  await exerciseSelect(page).selectOption({ label: 'プッシュアップ' });
  await exerciseSelect(page).selectOption({ label: 'ベンチプレス' });
  await groupSelect(page).selectOption({ label: 'すべての部位' });
  await exerciseSelect(page).selectOption({ label: '懸垂' });

  await expect(exerciseSelect(page).locator('option:checked')).toHaveText('懸垂');
  // セレクタが「懸垂」と言っている以上、統計も懸垂（12）でなければならない
  await expect(page.getByTestId('stat-best')).toHaveText('12');
});

// ── 保存 ────────────────────────────────────────────────────────────────────

test('6. 最後に見た部位と種目はリロードしても残り、Db には混ざらない', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  await groupSelect(page).selectOption({ label: '背中' });
  await exerciseSelect(page).selectOption({ label: '懸垂' });
  await flushToStorage(page);

  const stored = await page.evaluate(
    ({ dbKey, uiKey }) => ({
      db: localStorage.getItem(dbKey),
      ui: JSON.parse(localStorage.getItem(uiKey) ?? '{}'),
    }),
    { dbKey: STORAGE_KEY, uiKey: UI_KEY },
  );
  // ID は 12 文字の Crockford base32。数値で持つと JSON 往復で丸められる
  expect(stored.ui.progress_exercise, 'UI 専用キーに入る').toMatch(/^[0-9a-hjkmnp-tv-z]{12}$/);
  expect(stored.ui.progress_group).toMatch(/^[0-9a-hjkmnp-tv-z]{12}$/);
  expect(stored.db, 'Db には混ざらない（書き出しに載らない）').not.toContain('progress_exercise');

  await page.reload();
  await openProgress(page);
  await expect(groupSelect(page).locator('option:checked')).toHaveText('背中');
  await expect(exerciseSelect(page).locator('option:checked')).toHaveText('懸垂');
});

test('7. タブを往復しても選択は戻らない', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  await groupSelect(page).selectOption({ label: '背中' });
  await exerciseSelect(page).selectOption({ label: '懸垂' });

  // ★ `Progress` はタブ切替のたびに作り直される。コンテキストに載せず保存値から
  //   読み直しているので往復に耐える
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await openProgress(page);

  await expect(exerciseSelect(page).locator('option:checked')).toHaveText('懸垂');
});

test('8. 対象を変えても言語と表示数の設定が巻き添えで消えない', async ({ page }) => {
  // ★ `UiState` は 1 フィールドの deserialize が落ちると全体が飛ぶ。ID を
  //   `Option<String>` で受けているのはそれを避けるため
  // ★ 英語を選ぶ。Playwright の locale は ja-JP なので、日本語を押しても
  //   同値ガードで保存されず `lang` が書かれない
  await page.getByTestId('tab-settings').click();
  await page.getByTestId('settings-row-language').click();
  await page.getByTestId('lang-btn').and(page.locator('[data-lang="en"]')).click();
  await page.getByTestId('settings-back').click();

  await seedTwoGroups(page);
  await openProgress(page);
  await page.getByTestId('group-select').selectOption({ label: 'Back' });
  await flushToStorage(page);

  const ui = await uiState(page);
  expect(ui.progress_group).toMatch(/^[0-9a-hjkmnp-tv-z]{12}$/);
  expect(ui.lang, '言語の選択が巻き添えで消えていない').toBe('en');
});

// ── 宙に浮いた ID の受け皿 ──────────────────────────────────────────────────

test('9. 保存した種目の記録が消えると既定の種目へ倒れる', async ({ page }) => {
  await seedTwoGroups(page);
  await openProgress(page);

  await groupSelect(page).selectOption({ label: '背中' });
  await exerciseSelect(page).selectOption({ label: '懸垂' });
  await flushToStorage(page);
  const saved = (await uiState(page)).progress_exercise;

  // 懸垂の記録だけ消す。保存値はまだ懸垂を指したまま
  await page.evaluate((key) => {
    const db = JSON.parse(localStorage.getItem(key));
    const ex = db.exercises.find((e) => e.name === '懸垂');
    for (const s of Object.values(db.sessions)) {
      s.logs = s.logs.filter((l) => l.exercise_id !== ex.id);
    }
    localStorage.setItem(key, JSON.stringify(db));
  }, STORAGE_KEY);
  await page.reload();
  await openProgress(page);

  // ★ 候補から外れた ID は既定（記録がある先頭の種目）へ寄せ直される。
  //   ここが漏れると「select は先頭を表示しているのにグラフは空」の食い違いになる
  await expect(exerciseSelect(page).locator('option:checked')).toHaveText('ベンチプレス');
  await expect(groupSelect(page).locator('option:checked')).toHaveText('胸');
  // 保存値も既定へ揃う（宙に浮いた ID を残さない）
  await flushToStorage(page);
  expect((await uiState(page)).progress_exercise).not.toBe(saved);
});

// ★ 所属部位が消えた種目は `migrate` / `merge_db` が「宙に浮いた参照は残す」ので実在する。
//   部位に宙に浮いた ID を入れると、どの option にも当たらず部位セレクタが黙って
//   「すべての部位」へ落ち、`pick` と表示が食い違う。
test('13. 所属部位が消えた種目でもセレクタの表示と選択は食い違わない', async ({ page }) => {
  await seedTwoGroups(page);

  // ★ 先に flush する。起動直後の debounce 保存が残っていると、下の書き換えを
  //   400ms 後に「読み込んだ Db」で上書きされる
  await flushToStorage(page);
  await page.evaluate((key) => {
    const db = JSON.parse(localStorage.getItem(key));
    const ex = db.exercises.find((e) => e.name === '懸垂');
    db.groups = db.groups.filter((g) => g.id !== ex.group_id); // 背中ごと消す
    localStorage.setItem(key, JSON.stringify(db));
  }, STORAGE_KEY);
  await page.reload();
  await openProgress(page);

  // 部位セレクタに「背中」は無い。種目は「すべての部位」のときだけ並ぶ
  await expect(groupSelect(page).locator('option')).toHaveText(['すべての部位', '胸']);
  await groupSelect(page).selectOption({ label: 'すべての部位' });
  await exerciseSelect(page).selectOption({ label: '懸垂' });

  await expect(groupSelect(page).locator('option:checked')).toHaveText('すべての部位');
  await expect(exerciseSelect(page).locator('option:checked')).toHaveText('懸垂');
  // 記録そのものは参照できる（過去データを推移タブから見えなくしない）
  await expect(page.getByTestId('stat-best')).toHaveText('12');
});

test('10. 記録が 1 件も無いときは案内文を二重に出さない', async ({ page }) => {
  await openProgress(page);

  await expect(page.getByTestId('progress-empty')).toContainText('まだ記録がありません');
  // 両方「すべて」ではあるが、上の空状態が既に説明しているので重ねない
  await expect(page.getByTestId('progress-pick-hint')).toHaveCount(0);
  await expect(page.getByTestId('chart')).toHaveCount(0);
  // 中身ゼロの見出しをネイティブのピッカーに出さない
  await expect(page.getByTestId('exercise-select').locator('optgroup')).toHaveCount(0);
});
