//! トレーニングメニューの編集シート。**設定タブと記録タブの両方が開く。**
//!
//! ここに切り出してあるのは、入口が 2 つあるから:
//!
//! - 設定タブ（`views::settings`）の「＋ メニューを追加」と、既存メニューの行
//! - 記録タブ（`views::day`）の「この日をメニューにする」（その日の種目を初期選択にする）
//!
//! ★ **2 つ目の入口のために新しいシートを書かない。** 「名前 + 選択中 + 種目ピッカー」を
//!   もう 1 枚書くと、片方だけ直る事故が必ず起きる（保存の検証・トリム・並び順は全部
//!   同じ規則）。違いは `preset`（初期選択）と削除ボタンの有無だけなので引数で吸収する。
//!
//! ★ **同名を禁じない。** `core::merge_db` が「同名でも中身が違えば寄せない」と決めて
//!   いるので、同名のメニューは 2 台を混ぜれば普通に並ぶ。入口だけ塞いでも意味が無い。
//!   種目が同名を拒むのは `core::pin_presets` が「ちょうど 1 件」を要求するためで、
//!   メニューにはその制約が無い。

use leptos::prelude::*;
use web_sys::PointerEvent;

use crate::model::{Db, ExerciseId, GroupId, Routine, RoutineId};
use crate::reorder;
use crate::storage;

use super::drag::{
    self, Drag, EDGE_SCROLLING, PRESS, PRESS_DELAY_CARD, PRESS_SLOP_PX, Press, Scroller, capture,
    end_press, holds, measure_slots, release, start_edge_scroll,
};
use super::icon::{self, icon};
use super::{Sheet, kb_blur, kb_focus, use_dates, use_db, use_kb};

// ── Db の更新（純粋な操作だけをここに集める） ───────────────────────────────

/// ID は呼び側が採番して渡す（`views::settings` の `add_group` と同じ理由 —
/// この層は `web-sys` に触らない）。
fn add_routine(db: &mut Db, id: RoutineId, name: String, exercises: Vec<ExerciseId>) {
    // 末尾に足す。`Vec` の順が表示順なので `order` の付け替えは要らない
    db.routines.push(Routine {
        id,
        name,
        exercises,
    });
}

/// 名前と種目をまとめて置き換える。**シートの「保存」1 回で 1 度だけ呼ぶ。**
///
/// ★ 名前だけ / 種目だけの更新に分けない。分けると保存が 2 回の `Db` 更新になり、
///   間の 1 tick に「新しい名前 × 古い種目」という誰も入力していない状態が挟まる。
fn set_routine(db: &mut Db, id: RoutineId, name: String, exercises: Vec<ExerciseId>) {
    if let Some(r) = db.routines.iter_mut().find(|r| r.id == id) {
        r.name = name;
        r.exercises = exercises;
    }
}

/// メニューを消す。**物理削除でよい** — これを参照する記録は 1 つも無いので、
/// 消しても過去のログは 1 バイトも欠けない（種目のアーカイブとはここが違う）。
fn delete_routine(db: &mut Db, id: RoutineId) {
    db.routines.retain(|r| r.id != id);
}

/// 「選択中」の 1 行の DOM id。掴んだときに並び全体の箱を測るのに使う
/// （`views::drag::measure_slots`）。
///
/// ★ `ExerciseId` だけで一意になる。同じ種目は 2 度入らないので（`toggle` が push 前に
/// 位置を見る。取り込んだデータも `core::normalize_routines` が重複を落とす）、
/// これはそのまま `<For>` のキーにもなる。
fn picked_dom_id(ex: ExerciseId) -> String {
    format!("rtn-picked-{ex}")
}

/// メニュー 1 行に出す種目名。
///
/// ★ **アーカイブ済みも出す。** アーカイブは可逆なので、ここで隠すと「4 種目のはずが
///   3 つしか出ない」理由が画面から読めなくなる。存在しない種目（他端末のデータを
///   取り込むと起きる）だけは名前が引けないので落とす。
pub fn routine_exercise_names(db: &Db, id: RoutineId) -> Vec<String> {
    db.routine(id)
        .map(|r| {
            r.exercises
                .iter()
                .filter_map(|ex| db.exercise(*ex).map(|e| e.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// 「＋ この日をメニューにする」（adr/ux/save-a-day-as-a-routine.md）。
///
/// ★ **`<section class="day">` の外に置くこと。** `.add-wrap`（「種目を追加」）は
/// `position: sticky` で、その包含ブロックは `.day` である。つまり `.day` の中で
/// `.add-wrap` より後ろに置いた要素は、`.day` の末尾までスクロールしきるまで帯の下に
/// 潜りうる。`InstallBanner` がまったく同じ理由で `DayEditor` の外に置かれている
/// （`views::calendar` のコメント）。外に出せば帯は `.day` で止まるので構造的に干渉しない。
///
/// 必要な状態（`Db` と選択日）はコンテキストから自分で引くので、置き場所を選ばない。
#[component]
pub fn SaveDayAsRoutine() -> impl IntoView {
    let db = use_db();
    let dates = use_dates();

    // ★ シートは `Some(Vec)` を持つ。`bool` にして開くたびに `Db` から引き直す形にすると、
    //   シートを開いたまま裏で `Db` が動いたとき（デバウンス保存や取り込み）に
    //   選択が入れ替わる。**開いた瞬間の集合を握る**
    let open: RwSignal<Option<Vec<ExerciseId>>> = RwSignal::new(None);

    // その日の記録からメニューへ写せる種目。空なら導線ごと出さない。
    //
    // ★ カードの有無ではなく `core::day_exercises` を見る。カードには「種目を追加で
    //   出しただけでまだ何も打っていない」ものが混ざるので、そちらで判定すると
    //   **押しても何も保存されないリンク**が出る
    let exercises = Memo::new(move |_| {
        let date = dates.selected.get();
        db.with(|d| crate::core::day_exercises(d, date))
    });

    view! {
        {move || {
            let picked = exercises.get();
            (!picked.is_empty())
                .then(|| {
                    view! {
                        <div class="day-foot">
                            <button
                                class="link-btn"
                                data-testid="day-to-routine"
                                on:click=move |_| open.set(Some(picked.clone()))
                            >
                                "＋ この日をメニューにする"
                            </button>
                        </div>
                    }
                })
        }}

        <Sheet
            open=Signal::derive(move || open.get().is_some())
            on_close=Callback::new(move |_| open.set(None))
            title="この日をメニューにする".to_string()
            testid="day-routine-sheet"
            close_testid="day-routine-sheet-close"
        >
            // ★ 中身は設定タブと同じ `RoutineEditor`。もう 1 枚シートを書くと
            //   保存の検証・トリム・並び順が 2 実装に割れる
            {move || {
                open.get()
                    .map(|picked| {
                        view! {
                            <RoutineEditor
                                id=None
                                preset=picked
                                on_close=Callback::new(move |_| open.set(None))
                            />
                        }
                    })
            }}
        </Sheet>
    }
}

/// メニューの追加と編集。`id` が `None` なら新規。
///
/// ★ **追加と編集で 1 コンポーネントにする。** 違いは「初期値」と「削除ボタンの有無」
/// だけで、種目ピッカーという一番大きい部分が同じ。分けると片方だけ直る事故が起きる。
///
/// ★ **編集中の値は `Db` に書かず、ここのシグナルに持つ。** 1 タップごとに `Db` へ
/// 書くと、一覧（`RoutineBlock`）が作り直されるうえ、シートを閉じても取り消せない。
/// `Db` に触るのは「保存」と「削除」の 2 か所だけ。
///
/// ★ シートは常時マウントだが、このコンポーネント自体は呼び出し側の signal で
/// 作り直される（`Sheet` の中の `.map(..)`）ので、`with_untracked` で初期値を読んでよい。
#[component]
pub fn RoutineEditor(
    /// 編集するメニュー。`None` なら新規。
    id: Option<RoutineId>,
    /// 新規のときの初期選択。記録タブの「この日をメニューにする」がその日の種目を渡す。
    ///
    /// ★ `id` が `Some` のときは**無視する**。既存メニューを開いたのに別の集合が
    /// 入っていたら、何を編集しているのか分からなくなる。
    #[prop(optional)]
    preset: Vec<ExerciseId>,
    /// 保存 / 削除 / 取りやめのあとに呼ぶ。シートを閉じるのは呼び出し側の仕事。
    ///
    /// ★ 呼び出し側の signal（設定タブの `Editor` enum / 記録タブの `Option<Vec<_>>`）に
    /// 依存しないよう `Callback` で受ける。ここが片方の型を知っていると共有できない。
    #[prop(into)]
    on_close: Callback<()>,
) -> impl IntoView {
    let db = use_db();
    let kb = use_kb();

    let (name0, picked0) = match id {
        Some(id) => db
            .with_untracked(|d| d.routine(id).map(|r| (r.name.clone(), r.exercises.clone())))
            .unwrap_or_default(),
        None => (String::new(), preset),
    };

    let name = RwSignal::new(name0);
    // ★ 初期はタップ順。この並びがそのまま記録タブのカードの並びになるので、
    //   下の「選択中」をドラッグして組み替えられる
    let picked: RwSignal<Vec<ExerciseId>> = RwSignal::new(picked0);
    // 保存を止めた理由。**文言まで持つ**（bool にすると理由ごとに分岐が増える）
    let invalid: RwSignal<Option<&'static str>> = RwSignal::new(None);
    let confirming = RwSignal::new(false);
    // 「選択中」のドラッグ。全行が押しのけ量を読むのでここが持つ。
    // 長押し待ちは所有者を持たない `PRESS`（`views::drag`）に置く。
    let picked_drag: RwSignal<Option<Drag>> = RwSignal::new(None);

    // ★ このコンポーネントは**シートを閉じた瞬間に破棄される**（呼び出し側の signal が
    //   `None` になると `Sheet` の中の `.map(..)` ごと消える）。掴む途中で閉じると、
    //   生き残った長押しタイマーが破棄済みの `picked_drag` を触って panic する
    //   （wasm では unreachable ＝ アプリが死ぬ）。記録タブの `DayEditor` がタブ切替に
    //   対して同じことをしているのと同型で、畳んでから消える。
    //   `EDGE_SCROLLING` を下ろし忘れると再入防止のフラグが立ちっぱなしになり、
    //   開き直しても自動スクロールが二度と動かない
    on_cleanup(|| {
        end_press();
        EDGE_SCROLLING.set(false);
    });
    // シートの中だけで開いている部位。
    //
    // ★ **`OpenGroupCtx` を使わない。** あれはアプリ全体で 1 本の signal なので、
    //   ここで開いた部位が設定タブの種目一覧にも漏れる（逆も同じ）。
    // ★ **`Vec` にして複数開ける。** 設定タブの「1 つだけ開く」
    //   （adr/ux/menu-groups-as-single-open-accordion.md）とは別規則にする。あちらは
    //   種目マスタを 1 つずつ見に行く画面だが、こちらは**メニューを 1 本組む間に胸と脚を
    //   行き来する**のが普通の使い方で、排他にすると往復のたびに開き直すことになる。
    let open_groups: RwSignal<Vec<GroupId>> = RwSignal::new(Vec::new());

    let toggle = move |ex: ExerciseId| {
        picked.update(|p| match p.iter().position(|x| *x == ex) {
            Some(i) => {
                p.remove(i);
            }
            None => p.push(ex),
        });
        invalid.set(None);
    };

    // 「選択中」の並びを 1 つ動かす。**ドロップとキーボードの両方がここを通る。**
    //
    // ★ 落ちた先が元の位置なら signal に触らない。触ると `<For>` の keyed diff が
    //   空回りするだけで、タップで並びが変わらないことはこの分岐が保証する
    //   （閾値ではない。adr/ux/drag-to-reorder-in-record-tab.md）。
    let move_picked = move |from: usize, to: usize| {
        if from != to {
            picked.update(|p| reorder::move_item(p, from, to));
        }
    };

    // 「選択中」が 1 つでもあるか。
    //
    // ★ **`Memo` にする。** `picked.get()` を直に条件にすると、種目を 1 つ足すたびに
    //   `<ol>` ごと作り直され、`<For>` のキーが効かなくなる（＝ドラッグ中に DOM が
    //   入れ替わって pointer capture が落ちる経路が復活する）。真偽が変わったときだけ
    //   作り直せばよい。
    let has_picked = Memo::new(move |_| picked.with(|p| !p.is_empty()));

    let save = move |_| {
        // ★ 保存時の trim は他の 4 つのエディタ（部位 / 種目の追加・改名）と同じ。
        //   `core::normalize_routines` が trim しないのは**読み込み**の話で、あちらは
        //   自分が作ったのではないデータ（取り込んだファイル）を黙って書き換えないため。
        //   ここは利用者が入力欄を見たうえで「保存」を押した結果なので、揃えてよい
        let value = name.get_untracked().trim().to_string();
        let list = picked.get_untracked();
        // ★ 名前を必須にする。無名を許すと設定タブにも記録タブにも「（名前なし）」の行が
        //   複数並び、adr/ux/start-from-a-saved-routine.md が誤タップ対策の柱に据えた
        //   「行を見て選び分けられること」が種目名の 1 行だけに痩せる。
        //   種目・部位の追加も同じく名前を必須にしている
        if value.is_empty() {
            invalid.set(Some("メニュー名を入れてください"));
            return;
        }
        // ★ 種目が 0 個のメニューは保存させない。記録タブに出せないので、作れても
        //   「押せない行」が設定タブに残るだけになる（名前だけの状態を `normalize` が
        //   残すのは、既にあるデータを消さないためであって、新しく作るためではない）
        if list.is_empty() {
            invalid.set(Some("種目を 1 つ以上選んでください"));
            return;
        }
        match id {
            Some(id) => db.update(move |d| set_routine(d, id, value, list)),
            None => {
                let new_id = storage::alloc_id();
                db.update(move |d| add_routine(d, new_id, value, list));
            }
        }
        on_close.run(());
    };

    view! {
        // ★ **必須であることを label に書く。** 書かないと、押しても保存されない理由が
        //   押すまで分からない。この画面は種目ピッカーが縦を占めるので名前欄が埋もれる
        //   （他のエディタは入力欄 1 個なので気づける）
        <label class="field">
            <span>"メニュー名（必須）"</span>
            <input
                class="text-input"
                type="text"
                data-testid="routine-name-input"
                prop:value=move || name.get()
                on:focusin=move |_| kb_focus(kb)
                on:focusout=move |_| kb_blur(kb)
                on:input=move |ev| {
                    name.set(event_target_value(&ev));
                    invalid.set(None);
                }
            />
        </label>

        // 選んだ種目を並べる。ここが記録タブでのカードの並びになるので、順番が見えないと
        // 「どういう順で出るか」が保存するまで分からない。**番号を掴んで並べ替えられる。**
        //
        // ★ `<For key=…>` にすること。素の `map` だと `picked` が動くたびに `<ol>` ごと
        //   作り直され、ドラッグ中に DOM が差し替わって **pointer capture が落ちる**
        //   （adr/ux/drag-to-reorder-in-record-tab.md の「keyed diff は insertBefore で
        //   入れ直す」と同じ話が、こちらでは毎回確実に起きる）。
        {move || {
            has_picked
                .get()
                .then(|| {
                    view! {
                        <ol
                            class="rtn-picked"
                            data-testid="routine-picked"
                            // 押しのけの transition を生やす CSS フック。
                            // **ドラッグ中の親にだけ**（styles.css の該当節）
                            data-dragging=move || {
                                picked_drag.with(|d| d.is_some().then_some("true"))
                            }
                        >
                            <For
                                each=move || picked.get()
                                key=|ex| *ex
                                children=move |ex| {
                                    let label = move || {
                                        db.with(|d| d.exercise(ex).map(|e| e.name.clone()))
                                            .unwrap_or_else(|| "（削除された種目）".to_string())
                                    };
                                    // 模型上の位置。ドラッグ中も入れ替えないのでこれは動かない
                                    let slot = move || {
                                        picked.with(|p| p.iter().position(|x| *x == ex))
                                    };
                                    // ★ 番号は**画面で見えている位置**を出す。ドラッグ中は
                                    //   `Vec` を入れ替えないので模型の添字とずれ、そのまま
                                    //   描くと「2 番目に見えている行に 1 と書いてある」に
                                    //   なる。番号をハンドルにした前提（番号 ＝ 順番）が
                                    //   指を離すまで嘘になるので `visual_index` を通す。
                                    //   CSS の `counter()` ではこれが表現できない
                                    //   （中点を越えた瞬間に入れ替わってほしいが、
                                    //   counter は DOM 順でしか数えられない）ので、
                                    //   記録タブのセット番号と同じくテキストで描く
                                    let index = move || {
                                        let Some(i) = slot() else { return 0 };
                                        picked_drag
                                            .with(|d| d.as_ref().map_or(i, |d| d.seen_at(i))) + 1
                                    };
                                    // 長押しが満了したら、**そのときの指の位置**を基準に
                                    // 掴む。押した瞬間の位置を基準にすると、待っている
                                    // 間の 0〜10px ぶん掴んだ瞬間に行が跳ねる。測るのも
                                    // ここ（押した瞬間ではない）
                                    let arm = move || {
                                        let Some(p) = PRESS.get() else { return };
                                        // ★ `try_` で読む。長押しの途中でシートを閉じると
                                        //   このコンポーネントごと破棄され、タイマーだけが
                                        //   生き残る。`on_cleanup` で止めてはいるが、既に
                                        //   キューへ入った 1 発は止められない
                                        let Some((ids, here)) = picked
                                            .try_with_untracked(|p| {
                                                (
                                                    p.iter()
                                                        .map(|e| picked_dom_id(*e))
                                                        .collect::<Vec<_>>(),
                                                    p.iter().position(|x| *x == ex),
                                                )
                                            })
                                        else {
                                            return;
                                        };
                                        // ★ ここが記録タブと違う唯一の場所。この並びは
                                        //   `.sheet-body`（`overflow-y: auto`）の中にある
                                        //   ので、`window` のスクロール量を足しても
                                        //   1px も動かない
                                        let scroller = Scroller::of_id(&picked_dom_id(ex));
                                        let (Some(from), Some(slots)) = (
                                            here,
                                            measure_slots(&ids, &scroller),
                                        ) else {
                                            // 測れないまま始めると、押しのけ量が 1 つ
                                            // ずれた並びがそれらしく動く。掴まないほうがまし
                                            return;
                                        };
                                        if picked_drag
                                            .try_set(
                                                Some(
                                                    Drag::start(
                                                        p.pointer_id,
                                                        from,
                                                        p.last_y,
                                                        slots,
                                                        scroller,
                                                    ),
                                                ),
                                            )
                                            .is_none()
                                        {
                                            start_edge_scroll(picked_drag);
                                        }
                                    };
                                    let grab = move |ev: PointerEvent| {
                                        if picked_drag.with_untracked(Option::is_some)
                                            || !capture(&ev)
                                        {
                                            return;
                                        }
                                        // ★ **前の待ちが残っていても弾かずに畳んで立て直す。**
                                        //   `capture` が非 primary と左ボタン以外を落として
                                        //   いるので、ここまで来たのは必ず新しいジェスチャ。
                                        //   「残っていたら何もしない」形にすると、`pointerup`
                                        //   を 1 度でも取りこぼしたとき以後掴めなくなる
                                        end_press();
                                        // ★ capture は**待つ前に**取る。取らないと、待って
                                        //   いる 250ms の間に指がハンドルの外へ出たとき
                                        //   `pointermove` が届かず slop を判定できない
                                        let y = f64::from(ev.client_y());
                                        let timer = set_timeout_with_handle(
                                                arm,
                                                PRESS_DELAY_CARD,
                                            )
                                            .ok();
                                        PRESS.set(
                                            Some(Press {
                                                pointer_id: ev.pointer_id(),
                                                down_y: y,
                                                last_y: y,
                                                timer,
                                            }),
                                        );
                                    };
                                    let track = move |ev: PointerEvent| {
                                        if holds(picked_drag, &ev) {
                                            picked_drag
                                                .update(|d| {
                                                    if let Some(d) = d {
                                                        d.advance(f64::from(ev.client_y()));
                                                    }
                                                });
                                            return;
                                        }
                                        // まだ長押し待ち。動きすぎたらこのジェスチャは捨てる
                                        let Some(mut p) = PRESS
                                            .get()
                                            .filter(|p| p.pointer_id == ev.pointer_id())
                                        else {
                                            return;
                                        };
                                        p.last_y = f64::from(ev.client_y());
                                        if (p.last_y - p.down_y).abs() > PRESS_SLOP_PX
                                            && let Some(timer) = p.timer.take()
                                        {
                                            timer.clear();
                                        }
                                        PRESS.set(Some(p));
                                    };
                                    let drop_picked = move |ev: PointerEvent| {
                                        end_press();
                                        let Some(d) = picked_drag.get_untracked() else {
                                            return;
                                        };
                                        if d.pointer_id != ev.pointer_id() {
                                            return;
                                        }
                                        picked_drag.set(None);
                                        move_picked(d.from, d.to);
                                    };
                                    let cancel_picked = move |_: PointerEvent| {
                                        end_press();
                                        release(picked_drag);
                                    };
                                    // ドラッグの代わり。行の ✕ にフォーカスがあれば効く
                                    // （番号は `<button>` にできないので、既にフォーカス
                                    // できる要素へ載せる。記録タブと同じ作り）
                                    let nudge = move |ev: web_sys::KeyboardEvent| {
                                        let Some(up) = drag::alt_arrow(&ev) else { return };
                                        ev.prevent_default();
                                        let Some(from) = slot() else { return };
                                        let len = picked.with_untracked(Vec::len);
                                        move_picked(from, reorder::neighbor(from, up, len));
                                    };
                                    view! {
                                        <li
                                            id=picked_dom_id(ex)
                                            data-testid="routine-picked-row"
                                            data-drag=move || {
                                                picked_drag
                                                    .with(|d| {
                                                        (d.as_ref()?.from == slot()?)
                                                            .then_some("lift")
                                                    })
                                            }
                                            style:transform=move || {
                                                picked_drag
                                                    .with(|d| d.as_ref()?.transform(slot()?))
                                            }
                                            on:keydown=nudge
                                        >
                                            // ★ **掴む場所は行のほぼ全部**（記録タブの
                                            //   `.card-head` と同じ考え方）。番号だけを
                                            //   ハンドルにしていたが、16px の標的では
                                            //   カードのドラッグと操作感が揃わない。
                                            //   adr/ux/routine-editor-drag-and-accordion.md
                                            //
                                            // ★ **✕ をこの中に入れない。** 入れると
                                            //   「外すつもりのタップでドラッグが始まる」を
                                            //   `stop_propagation` で消して回ることになる。
                                            //   兄弟にすれば `pointerdown` がそもそも
                                            //   ここへ来ないので、構造で保証できる
                                            //   （`.card-head` に削除ボタンが 1 つも
                                            //   無いのと同じ形）。
                                            //
                                            // ★ `<button>` にはできない。中に 2 つの
                                            //   `<span>` を並べるだけなら可能だが、行全体が
                                            //   押せるコントロールに見えると「押すと何が
                                            //   起きるのか」が嘘になる（起きるのは
                                            //   長押しの並べ替えだけ）。キーボードからの
                                            //   並び替えは `<li>` の `on:keydown` が受ける
                                            <div
                                                class="rtn-grab"
                                                data-testid="routine-handle"
                                                on:pointerdown=grab
                                                on:pointermove=track
                                                on:pointerup=drop_picked
                                                on:pointercancel=cancel_picked
                                                on:lostpointercapture=cancel_picked
                                            >
                                                <span class="rtn-no" data-testid="routine-no">
                                                    {index}
                                                </span>
                                                <span
                                                    class="rtn-name"
                                                    data-testid="routine-picked-name"
                                                >
                                                    {label}
                                                </span>
                                            </div>
                                            // ★ ここは ✕ のまま。**trash にしない** —
                                            //   これは「このメニューから外す」で種目自体は
                                            //   1 つも消えないので、trash だと種目を削除した
                                            //   ように読める（下の「このメニューを削除」との
                                            //   違いが消える）
                                            <button
                                                class="icon-btn"
                                                aria-label=move || format!("{} を外す", label())
                                                data-testid="routine-remove"
                                                on:click=move |_| toggle(ex)
                                            >
                                                {icon(icon::X)}
                                            </button>
                                        </li>
                                    }
                                }
                            />
                        </ol>
                    }
                })
        }}

        // 部位ごとの種目ピッカー。押すと選択が入れ替わる。
        // ★ アーカイブ済みは出さない（記録タブの「種目を追加」と同じ規則）。既にメニューへ
        //   入っているアーカイブ済み種目は上の「選択中」に出るので、外すことはできる
        //
        // ★ **折りたたむ。既定は全部閉。** 開きっぱなしだと 6 部位 28 種目が縦に伸びて、
        //   組み終わったメニューを見返すには毎回スクロールし切ることになる（保存帯を
        //   sticky にしたのと同じ問題が「選択中」に対して起きる）。
        // ★ 見た目とフックは設定タブの `GroupBlock` に揃える（`.grp-toggle` +
        //   `aria-expanded` でシェブロンを回す。`<details>`/`<summary>` は使わない）。
        //   `GroupBlock` 自体を再利用しないのは、あれが `views::settings` の private な
        //   `Editor` enum を prop に取っていて、鉛筆（部位の改名）まで一緒に付いてくるから。
        //   ここに改名の入口は要らない。
        {move || {
            db.with(|d| {
                let mut groups = d.groups.clone();
                groups.sort_by_key(|g| g.order);
                groups
                    .into_iter()
                    .map(|g| {
                        let gid = g.id;
                        let mut exercises: Vec<_> = d
                            .exercises
                            .iter()
                            .filter(|e| e.group_id == gid && !e.archived)
                            .cloned()
                            .collect();
                        exercises.sort_by_key(|e| e.order);
                        let count = exercises.len();
                        // ★ `open_groups` を読むのは**この内側の closure だけ**にする。
                        //   外側（`db.with(..)` のブロック）で読むと、1 部位開くたびに
                        //   ピッカー全体が作り直される
                        let open = move || open_groups.with(|v| v.contains(&gid));
                        view! {
                            <section class="sheet-group rtn-group">
                                // ★ **`<h3>` は残す。** ここは畳む前から部位の見出しで、
                                //   `<button>` に置き換えると 6 部位ぶんの見出しが a11y
                                //   ツリーから消える（`views::settings` の `GroupBlock` に
                                //   見出しが無いのは、あちらが `.card-head` の中に鉛筆と
                                //   並べているから）。`<button>` は phrasing content なので
                                //   `<h3>` の中に置けて、WAI-ARIA APG のアコーディオンも
                                //   「見出し > ボタン」を求めている
                                <h3>
                                <button
                                    class="grp-toggle"
                                    data-testid="routine-group-toggle"
                                    aria-expanded=move || if open() { "true" } else { "false" }
                                    on:click=move |_| {
                                        open_groups
                                            .update(|v| match v.iter().position(|x| *x == gid) {
                                                Some(i) => {
                                                    v.remove(i);
                                                }
                                                None => v.push(gid),
                                            })
                                    }
                                >
                                    // 開いた状態は CSS で 90 度回す（chevron-down を持たない）
                                    {icon(icon::CHEVRON_RIGHT)}
                                    <span
                                        class="dot"
                                        style=format!("--dot:{}", g.color)
                                    ></span>
                                    <span class="grp-name" data-testid="routine-group-name">
                                        {g.name}
                                    </span>
                                    <span class="grp-count muted">
                                        {format!("{count} 種目")}
                                    </span>
                                </button>
                                </h3>
                                {move || {
                                    open()
                                        .then(|| {
                                            let list = exercises.clone();
                                            view! {
                                                <div class="pick-list">
                                                    {list
                                                        .into_iter()
                                                        .map(|e| {
                                                            let ex = e.id;
                                                            view! {
                                                                <button
                                                                    class="pick"
                                                                    class:added=move || {
                                                                        picked.with(|p| p.contains(&ex))
                                                                    }
                                                                    data-testid="routine-pick"
                                                                    on:click=move |_| toggle(ex)
                                                                >
                                                                    {e.name}
                                                                </button>
                                                            }
                                                        })
                                                        .collect::<Vec<_>>()}
                                                </div>
                                            }
                                        })
                                }}
                            </section>
                        }
                    })
                    .collect::<Vec<_>>()
            })
        }}

        {move || {
            id.map(|id| {
                view! {
                    <div class="sheet-actions">
                        // 破壊的操作は静止時に警告色を持たない（adr/ux/destructive-affordance-quiet-at-rest.md）。
                        // ★ **アイコンを足す。** 文字だけのリンクがシートの一番下、
                        //   保存帯の直前に 1 行あるだけで、上に並ぶ種目ボタンの列に
                        //   埋もれて「どこで消せるのか」が読み取れなかった。
                        //   trash の線色は `currentColor` なので、この行の色
                        //   （`.danger`）は 1 文字も変えていない
                        <button
                            class="link-btn danger"
                            data-testid="delete-routine"
                            on:click=move |_| confirming.set(true)
                        >
                            {icon(icon::TRASH_2)}
                            "このメニューを削除"
                        </button>
                    </div>
                    // ★ 確認を挟む。組んだ種目の並びは元に戻せず、undo も無い
                    //   （セット削除に確認が無いのは 1 行だけの損失だから。
                    //   adr/ux/set-delete-without-confirmation.md）。部位削除と同じ形にする
                    {move || {
                        confirming
                            .get()
                            .then(|| {
                                view! {
                                    <div class="warn-box">
                                        <p>"このメニューを削除します"</p>
                                        // 一番怖いのは「記録も消えるのでは」なので先に否定する
                                        <p class="muted">
                                            "記録は 1 件も消えません（メニューは種目の組み合わせを覚えているだけです）"
                                        </p>
                                        <div class="sheet-actions">
                                            <button
                                                class="primary"
                                                data-testid="delete-routine-confirm"
                                                on:click=move |_| {
                                                    db.update(move |d| delete_routine(d, id));
                                                    on_close.run(());
                                                }
                                            >
                                                "削除する"
                                            </button>
                                            <button
                                                class="link-btn"
                                                on:click=move |_| confirming.set(false)
                                            >
                                                "やめる"
                                            </button>
                                        </div>
                                    </div>
                                }
                            })
                    }}
                }
            })
        }}

        // ★ **保存はシート内で sticky にする。** この画面は種目ピッカーが 6 部位ぶん
        //   縦に伸びるので、通常フローに置くと「保存するにはピッカーを全部スクロールし切る」
        //   になる。他のエディタ（部位 / 種目）は入力欄 1 個で下端が見えているから
        //   `.sheet-actions` のままでよい — 長いのはこの 1 枚だけ。
        //
        // ★ **DOM の最後に置く。** 前に置くと、後ろに続く削除ボタンや確認ボックスが
        //   スクロール途中で帯の下に潜る（記録タブの `.add-wrap` と同じ罠）。
        //
        // ★ 保存できない理由もこの中に出す。ボタンの隣でないと、押した場所と
        //   「なぜ効かなかったか」が画面の別の場所に離れる（この画面の高さでは画面外）。
        <div class="rtn-save" data-testid="routine-save-bar">
            {move || {
                invalid
                    .get()
                    .map(|reason| {
                        view! { <p class="rtn-invalid" data-testid="routine-invalid">{reason}</p> }
                    })
            }}
            <button class="primary" data-testid="routine-save" on:click=save>
                "保存"
            </button>
        </div>
    }
}
