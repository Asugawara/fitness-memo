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

use crate::model::{Db, ExerciseId, Routine, RoutineId};
use crate::storage;

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
    // ★ タップ順を保つ。この並びがそのまま記録タブのカードの並びになる
    let picked: RwSignal<Vec<ExerciseId>> = RwSignal::new(picked0);
    // 保存を止めた理由。**文言まで持つ**（bool にすると理由ごとに分岐が増える）
    let invalid: RwSignal<Option<&'static str>> = RwSignal::new(None);
    let confirming = RwSignal::new(false);

    let toggle = move |ex: ExerciseId| {
        picked.update(|p| match p.iter().position(|x| *x == ex) {
            Some(i) => {
                p.remove(i);
            }
            None => p.push(ex),
        });
        invalid.set(None);
    };

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

        // 選んだ種目を**タップ順で**並べる。ここが記録タブでのカードの並びになるので、
        // 順番が見えないと「どういう順で出るか」が保存するまで分からない
        {move || {
            let list = picked.get();
            (!list.is_empty())
                .then(|| {
                    view! {
                        <ol class="rtn-picked" data-testid="routine-picked">
                            {list
                                .into_iter()
                                .map(|ex| {
                                    let label = db
                                        .with(|d| d.exercise(ex).map(|e| e.name.clone()))
                                        .unwrap_or_else(|| "（削除された種目）".to_string());
                                    view! {
                                        <li>
                                            <span>{label.clone()}</span>
                                            <button
                                                class="icon-btn"
                                                aria-label=format!("{label} を外す")
                                                data-testid="routine-remove"
                                                on:click=move |_| toggle(ex)
                                            >
                                                {icon(icon::X)}
                                            </button>
                                        </li>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </ol>
                    }
                })
        }}

        // 記録タブの「種目を追加」シートと同じ部位ごとの一覧。押すと選択が入れ替わる。
        // ★ アーカイブ済みは出さない（あちらと同じ規則）。既にメニューへ入っている
        //   アーカイブ済み種目は上の「選択中」に出るので、外すことはできる
        {move || {
            db.with(|d| {
                let mut groups = d.groups.clone();
                groups.sort_by_key(|g| g.order);
                groups
                    .into_iter()
                    .map(|g| {
                        let mut exercises: Vec<_> = d
                            .exercises
                            .iter()
                            .filter(|e| e.group_id == g.id && !e.archived)
                            .cloned()
                            .collect();
                        exercises.sort_by_key(|e| e.order);
                        view! {
                            <section class="sheet-group">
                                <h3 style=format!("--dot:{}", g.color)>{g.name}</h3>
                                <div class="pick-list">
                                    {exercises
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
                        // 破壊的操作は静止時に警告色を持たない（adr/ux/destructive-affordance-quiet-at-rest.md）
                        <button
                            class="link-btn danger"
                            data-testid="delete-routine"
                            on:click=move |_| confirming.set(true)
                        >
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
