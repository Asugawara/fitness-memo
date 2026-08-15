//! 設定タブ。トレーニングメニュー・部位グループ・種目の管理。
//!
//! 設計上の要点:
//!
//! - **部位の削除ガードはアーカイブ済み種目も所属種目に数える。** ここが緩むと
//!   アーカイブ済み種目の `group_id` が宙に浮き、カレンダーのドット色・部位別グラフ・
//!   今日タブの部位チップが**過去分だけ**壊れる。画面に出ていない種目が理由で削除できない
//!   のは分からないので、文言に件数を出す（「アーカイブ済み種目が 2 件あります」）
//! - **種目は「指標の種類」を持たない。** 加重 / 自重 / 時間の区別は種目名から読めるので、
//!   選ばせる意味が無かった。指標は `core::set_volume` の単一式に統一され、どの軸で
//!   見るかは推移タブの `Metric` が決める
//! - **アーカイブは論理削除。** 過去ログの `exercise_id` 参照を保つのが目的。アーカイブ済み
//!   種目は「種目を追加」シートには出ないが、推移タブの種目セレクタからは参照
//!   できる（過去データが参照不能になるのを防ぐ）
//! - **部位は 1 つずつ開く。** 一覧は既定で全部閉じていて、ヘッダを押すとその部位の種目
//!   だけが出る。名前と色の編集は右端の鉛筆から。ヘッダのタップ標的を「開閉」と「編集」の
//!   2 つに絞るのが要点で、44px のボタンを何個も並べると部位を見渡すという一覧本来の
//!   役目が潰れる（adr/ux/menu-groups-as-single-open-accordion.md）
//! - **並び替えの手段を持たない。** `Group.order` / `Exercise.order` はプリセットの宣言順
//!   → 以後は追加順で固定される。この order は設定タブの表示順だけでなく、記録タブ
//!   「種目を追加」シートの部位セクション順とその中の種目順、推移タブの種目セレクタも
//!   決めている（adr/ux/menu-groups-as-single-open-accordion.md）
//! - 編集フォームは一覧の外に置いた 1 枚のシートに集約する。一覧の中に入力欄を置くと、
//!   1 文字打つたびに `Db` が動いて一覧ごと作り直され、編集中の文字列が消える

use leptos::prelude::*;

use crate::model::{Db, Exercise, ExerciseId, Group, GroupId, RoutineId};
use crate::storage;

use super::help::InstallHelpLink;
use super::icon::{self, icon};
use super::routine::{RoutineEditor, routine_exercise_names};
use super::{
    SettingsPage, Sheet, kb_blur, kb_focus, scroll_into_view_if_needed, use_db, use_kb,
    use_open_group, use_settings_page,
};

/// 部位を追加するときの既定色。プリセットの 6 色を順に回す。
const COLOR_CHOICES: [&str; 6] = [
    "#e0524a", "#2f7fd1", "#e0912a", "#7a56c9", "#2fa06a", "#6b7280",
];

// ── Db の更新（純粋な操作だけをここに集める） ───────────────────────────────

fn ordered_group_ids(db: &Db) -> Vec<GroupId> {
    let mut list: Vec<(u32, GroupId)> = db.groups.iter().map(|g| (g.order, g.id)).collect();
    list.sort_unstable();
    list.into_iter().map(|(_, id)| id).collect()
}

/// その部位の**アーカイブされていない**種目を表示順に。
fn ordered_exercise_ids(db: &Db, group: GroupId) -> Vec<ExerciseId> {
    let mut list: Vec<(u32, ExerciseId)> = db
        .exercises
        .iter()
        .filter(|e| e.group_id == group && !e.archived)
        .map(|e| (e.order, e.id))
        .collect();
    list.sort_unstable();
    list.into_iter().map(|(_, id)| id).collect()
}

/// アーカイブ済み種目を「部位の並び順 → 部位内の順」で。
///
/// 部位が見つからない種目（本来は削除ガードで起きない）は末尾に出す。どこからも
/// 見えなくなるより、ここに出して戻せるほうがよい。
fn archived_ids(db: &Db) -> Vec<ExerciseId> {
    let mut list: Vec<(u32, u32, ExerciseId)> = db
        .exercises
        .iter()
        .filter(|e| e.archived)
        .map(|e| {
            (
                db.group(e.group_id).map_or(u32::MAX, |g| g.order),
                e.order,
                e.id,
            )
        })
        .collect();
    list.sort_unstable();
    list.into_iter().map(|(_, _, id)| id).collect()
}

fn apply_group_order(db: &mut Db, ordered: &[GroupId]) {
    for (i, id) in ordered.iter().enumerate() {
        if let Some(g) = db.groups.iter_mut().find(|g| g.id == *id) {
            g.order = i as u32;
        }
    }
}

/// その部位の末尾に付けるための `order`。**アーカイブ済みも含めて**最大値の次を返す。
///
/// ★ 番号は詰め直さない。かつては「非アーカイブ → アーカイブ済み」で 0.. に振り直して
///   いたが、そうするとアーカイブした瞬間にその種目の元の位置が失われ、戻したときに
///   必ず末尾へ落ちる。並び替えの UI が無くなった今、そこでずれると直す手段が無い
///   （記録タブ「種目を追加」シートの並びも同じ `order` で決まる）。飛び番が空いても
///   `order` は相対順序にしか使わないので害はない。
fn next_order(db: &Db, group: GroupId) -> u32 {
    db.exercises
        .iter()
        .filter(|e| e.group_id == group)
        .map(|e| e.order)
        .max()
        .map_or(0, |m| m.saturating_add(1))
}

fn rename_group(db: &mut Db, id: GroupId, name: &str) {
    if let Some(g) = db.groups.iter_mut().find(|g| g.id == id) {
        g.name = name.to_string();
    }
}

fn set_group_color(db: &mut Db, id: GroupId, color: &str) {
    if let Some(g) = db.groups.iter_mut().find(|g| g.id == id) {
        g.color = color.to_string();
    }
}

/// ID は呼び側（イベントハンドラ）が採番して渡す。
///
/// `storage::alloc_id` は `web-sys` に触るので、ここで呼ぶと「Db の更新は純粋な操作
/// だけを集める」というこのセクションの前提が崩れる。
fn add_group(db: &mut Db, id: GroupId, name: String, color: String) {
    let order = db.groups.len() as u32;
    db.groups.push(Group {
        id,
        name,
        color,
        order,
    });
}

/// **アーカイブ済み種目も所属種目に数える**（`Db::exercise_ids_of_group` がアーカイブ込み）。
fn delete_group(db: &mut Db, id: GroupId) {
    if !db.exercise_ids_of_group(id).is_empty() {
        return;
    }
    db.groups.retain(|g| g.id != id);
    let ordered = ordered_group_ids(db);
    apply_group_order(db, &ordered);
}

fn rename_exercise(db: &mut Db, id: ExerciseId, name: &str) {
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.name = name.to_string();
    }
}

/// ID は呼び側が採番して渡す（[`add_group`] と同じ理由）。
fn add_exercise(db: &mut Db, id: ExerciseId, group: GroupId, name: String) {
    let order = next_order(db, group);
    db.exercises.push(Exercise {
        id,
        name,
        group_id: group,
        order,
        archived: false,
    });
}

fn set_exercise_group(db: &mut Db, id: ExerciseId, group: GroupId) {
    let Some(from) = db.exercise(id).map(|e| e.group_id) else {
        return;
    };
    if from == group {
        return;
    }
    // 移動先の末尾へ。移動元に空く飛び番は詰めない（[`next_order`] の注記を参照）
    let order = next_order(db, group);
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.group_id = group;
        e.order = order;
    }
}

/// アーカイブ = 論理削除。過去ログの `exercise_id` 参照を保つ。
///
/// ★ `order` には触らない。アーカイブ中も元の位置を持ち続けることで、戻したときに
///   元いた場所へ帰る。並び替えの UI が無い以上、ここで末尾へ落とすと二度と直せない。
fn set_archived(db: &mut Db, id: ExerciseId, archived: bool) {
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.archived = archived;
    }
}

// ── シート ──────────────────────────────────────────────────────────────────

/// シートで開いているもの。
///
/// 編集フォームを一覧の外の 1 枚に畳むことで、一覧が入力のたびに作り直されて編集中の
/// 文字列が消えるのを構造的に防ぐ（このクロージャは `editor` だけを購読する）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Editor {
    Group(GroupId),
    NewGroup,
    Exercise(ExerciseId),
    NewExercise(GroupId),
    Routine(RoutineId),
    NewRoutine,
}

/// シートの見出し。閉じている（`None`）ときは空文字。
///
/// ★ シートは常時マウントなので、閉じている間もこれが呼ばれる。空文字で構わない
/// （`aria-label` も見出しも、閉じた `<dialog>` は支援技術から辿れない）。
fn editor_title(target: Option<Editor>) -> &'static str {
    match target {
        Some(Editor::Group(_)) => "部位を編集",
        Some(Editor::NewGroup) => "部位を追加",
        Some(Editor::Exercise(_)) => "種目を編集",
        Some(Editor::NewExercise(_)) => "種目を追加",
        Some(Editor::Routine(_)) => "メニューを編集",
        Some(Editor::NewRoutine) => "メニューを追加",
        None => "",
    }
}

/// 部位カードの DOM id。開いた部位を画面に入れるために引く。
fn grp_dom_id(group: GroupId) -> String {
    format!("grp-{group}")
}

/// 設定トップの節 1 行。押すと節へ入る（または対応するシートを開く）。
///
/// ★ 右端の数は「その節に何件あるか」。0 件でも出す — 「まだ 0 件」が読めることが、
/// 入って初めて空だと分かるより速い。`None` を渡す行（シートを開くだけの行）は出さない。
fn section_row(
    label: &'static str,
    count: Option<Signal<String>>,
    testid: &'static str,
    open: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <button class="row" data-testid=testid on:click=move |_| open()>
            <span class="row-label">{label}</span>
            {count
                .map(|c| view! { <span class="row-count muted">{move || c.get()}</span> })}
            // シェブロンは「押すと先がある」ことだけを示す。部位ヘッダのものと同じ向き
            {icon(icon::CHEVRON_RIGHT)}
        </button>
    }
}

/// 節の見出し + 「‹ 設定」。**節の中では h1 がその節名**になる（h1 は常に画面に 1 つ）。
fn back_head(title: &'static str, back: impl Fn() + 'static) -> impl IntoView {
    view! {
        <header class="settings-head">
            <button class="icon-btn" aria-label="設定へ戻る" data-testid="settings-back"
                on:click=move |_| back()>
                {icon(icon::CHEVRON_LEFT)}
            </button>
            <h1>{title}</h1>
        </header>
    }
}

/// 選択肢ボタン 1 個。今は部位の選択にだけ使う。
fn opt_button(
    label: String,
    testid: &'static str,
    selected: impl Fn() -> bool + Send + Sync + 'static,
    pick: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <button
            class="opt"
            class:on=move || selected()
            data-testid=testid
            on:click=move |_| pick()
        >
            {label}
        </button>
    }
}

// ── 画面 ────────────────────────────────────────────────────────────────────

#[component]
pub fn Settings() -> impl IntoView {
    let db = use_db();
    let editor: RwSignal<Option<Editor>> = RwSignal::new(None);
    let backup_open = RwSignal::new(false);

    // 開いている部位。**1 つだけ開く**（adr/ux/menu-groups-as-single-open-accordion.md）。シグナル自体は `App` が持つ
    // （タブを往復しても閉じないため。理由は `OpenGroupCtx` の doc を参照）。
    // `Db` とは無関係なので、改名や色変更で閉じることもない
    let open_group = use_open_group();

    // ★ 開いた部位は必ず画面に入れる。`open_group.set(..)` するだけでは、一覧の**下**に
    //   あるアーカイブ済みセクションから戻したときや、末尾に作られた新規部位が
    //   「画面の外で開く」だけになり、自動展開が塞いだはずの「どこへ行ったか分からない」
    //   がそのまま残る。`block: nearest` なので、タップして開いただけの部位は動かない
    Effect::new(move |_| {
        if let Some(id) = open_group.get() {
            scroll_into_view_if_needed(grp_dom_id(id));
        }
    });

    let group_ids = Memo::new(move |_| db.with(ordered_group_ids));
    let archived = Memo::new(move |_| db.with(archived_ids));
    // メニューは `Vec` の順がそのまま表示順（`order` を持たない）
    let routine_ids =
        Memo::new(move |_| db.with(|d| d.routines.iter().map(|r| r.id).collect::<Vec<_>>()));

    // 開いているページ。シグナル自体は `App` が持つ（`OpenGroupCtx` と同じ理由 —
    // タブを往復してもトップへ戻されない）
    let page = use_settings_page();
    // 節へ入る / 戻る。
    //
    // ★ 編集シートを閉じるのは**保険**。シートは modal（`show_modal()`）で開いている間は
    //   背景が inert なので、この経路は今の UI からは踏めない（E2E で確認した）。それでも
    //   書くのは、ページを替える口がここ 1 つだからで、非 modal な導線を足した誰かが
    //   「別の節に居るのに前の節の編集シートが載っている」を作らずに済む。
    let go = move |to: SettingsPage| {
        editor.set(None);
        page.set(to);
    };

    view! {
        <section class="settings" data-testid="screen-settings">
            // プリセットの投入は初回起動時に storage::load が済ませる。再投入の導線は持たない
            // （改名済みプリセットが別種目として復活する挙動があり、得より害が大きかった）
            //
            // ★ この画面は**節の一覧が入口**（adr/ux/settings-as-a-list-of-sections.md）。
            //   4 つを 1 画面に縦積みしていた頃は、メニューを 1 本足すのに 6 部位 28 種目の
            //   一覧を跨いでスクロールすることになっていた。
            //   h1 は常に 1 つ（記録タブと同じ規則、adr/ux/focus-ring-and-heading-order.md）。
            //   節の中では h1 がその節名になり、左上に「‹ 設定」が出る
            {move || match page.get() {
                SettingsPage::Root => {
                    view! {
                        <header class="settings-head">
                            <h1>"設定"</h1>
                        </header>

                        <div class="rows" data-testid="settings-rows">
                            // ★ 先頭に置く。データを失う前に見つけてもらう必要があるので、
                            //   ここだけは他の節より上（旧レイアウトの sticky と同じ意図）
                            {section_row(
                                "データの書き出し / 読み込み",
                                None,
                                "open-backup",
                                move || {
                                    editor.set(None);
                                    backup_open.set(true);
                                },
                            )}
                            {section_row(
                                "トレーニングメニュー",
                                Some(Signal::derive(move || routine_ids.get().len().to_string())),
                                "settings-row-routines",
                                move || go(SettingsPage::Routines),
                            )}
                            {section_row(
                                "種目",
                                Some(
                                    Signal::derive(move || {
                                        db.with(|d| d.exercises.iter().filter(|e| !e.archived).count())
                                            .to_string()
                                    }),
                                ),
                                "settings-row-exercises",
                                move || go(SettingsPage::Exercises),
                            )}
                            // 手順シートを開くだけなので、節ではなく行として並べる
                            <InstallHelpLink />
                        </div>
                    }
                        .into_any()
                }
                SettingsPage::Routines => {
                    view! {
                        {back_head("トレーニングメニュー", move || go(SettingsPage::Root))}

                        <For
                            each=move || routine_ids.get()
                            key=|id| *id
                            children=move |id| view! { <RoutineBlock routine=id editor=editor /> }
                        />
                        {move || {
                            routine_ids
                                .get()
                                .is_empty()
                                .then(|| {
                                    view! {
                                        <p class="settings-note muted" data-testid="routines-empty">
                                            "よくやる種目の組み合わせに名前を付けておくと、記録タブで 1 タップで呼び出せます"
                                        </p>
                                    }
                                })
                        }}
                        <div class="add-wrap">
                            <button
                                class="secondary"
                                data-testid="settings-add-routine"
                                on:click=move |_| editor.set(Some(Editor::NewRoutine))
                            >
                                "＋ メニューを追加"
                            </button>
                        </div>
                    }
                        .into_any()
                }
                SettingsPage::Exercises => {
                    view! {
                        {back_head("種目", move || go(SettingsPage::Root))}

                        <p class="settings-note muted">
                            "アーカイブした種目は「種目を追加」に出なくなりますが、過去の記録は残り推移タブから参照できます"
                        </p>
                        <For
                            each=move || group_ids.get()
                            key=|id| *id
                            children=move |id| {
                                view! { <GroupBlock group=id editor=editor open_group=open_group /> }
                            }
                        />

                        <div class="add-wrap">
                            <button
                                class="secondary"
                                data-testid="settings-add-group"
                                on:click=move |_| editor.set(Some(Editor::NewGroup))
                            >
                                "＋ 部位を追加"
                            </button>
                        </div>

                        {move || {
                            let ids = archived.get();
                            (!ids.is_empty())
                                .then(|| view! { <ArchivedSection ids=ids open_group=open_group /> })
                        }}
                    }
                        .into_any()
                }
            }}

            <super::backup::BackupSheet open=backup_open />

            // 見出しと中身はどちらも `editor` から引く。★ シートは常時マウントなので、
            // 閉じている間（`editor` が None）は見出しが空文字・中身が無しになる
            <Sheet
                open=Signal::derive(move || editor.get().is_some())
                on_close=Callback::new(move |_| editor.set(None))
                title=Signal::derive(move || editor_title(editor.get()).to_string())
                testid="settings-sheet"
                close_testid="settings-sheet-close"
            >
                {move || {
                    editor
                        .get()
                        .map(|target| match target {
                            Editor::Group(id) => {
                                view! { <GroupEditor id=id editor=editor open_group=open_group /> }
                                    .into_any()
                            }
                            Editor::NewGroup => {
                                view! { <NewGroupEditor editor=editor open_group=open_group /> }
                                    .into_any()
                            }
                            Editor::Exercise(id) => {
                                view! {
                                    <ExerciseEditor id=id editor=editor open_group=open_group />
                                }
                                    .into_any()
                            }
                            Editor::NewExercise(group) => {
                                view! {
                                    <NewExerciseEditor
                                        group=group
                                        editor=editor
                                        open_group=open_group
                                    />
                                }
                                    .into_any()
                            }
                            Editor::Routine(id) => {
                                view! {
                                    <RoutineEditor
                                        id=Some(id)
                                        on_close=Callback::new(move |_| editor.set(None))
                                    />
                                }
                                    .into_any()
                            }
                            Editor::NewRoutine => {
                                view! {
                                    <RoutineEditor
                                        id=None
                                        on_close=Callback::new(move |_| editor.set(None))
                                    />
                                }
                                    .into_any()
                            }
                        })
                }}
            </Sheet>
        </section>
    }
}

/// メニュー 1 本ぶん。**行全体が編集シートを開くボタン。**
///
/// ★ 部位のようなアコーディオンにしない。部位に開閉が要るのは 28 種目を畳むためで、
/// メニューは 4〜6 種目なので 2 段の 1 行に収まる。開いても中に操作は無いので、
/// 「開く」と「編集」を分けるとタップが 1 つ増えるだけになる。
#[component]
fn RoutineBlock(routine: RoutineId, editor: RwSignal<Option<Editor>>) -> impl IntoView {
    let db = use_db();

    let name = Memo::new(move |_| {
        db.with(|d| d.routine(routine).map(|r| r.name.clone()))
            .unwrap_or_default()
    });
    let names = Memo::new(move |_| db.with(|d| routine_exercise_names(d, routine)));
    // ★ 件数は**記録タブで実際に開く数**を出す。`names` の長さ（保存されている種目の数）を
    //   出すと、アーカイブ済みを 1 つ含むだけで「2 種目」と書いてあるのに 1 枚しか開かない
    //   ことになる。判定は `core::expandable_count` に聞く（同じ条件を書き写さない）
    let open_count = Memo::new(move |_| db.with(|d| crate::core::expandable_count(d, routine)));
    // 名前は出すのに開かない種目（＝アーカイブ済み）の数。0 なら注記を出さない
    let hidden = Memo::new(move |_| names.get().len().saturating_sub(open_count.get()));

    view! {
        <section class="card rtn" data-testid="routine-item">
            <button
                class="rtn-open"
                data-testid="routine-open"
                on:click=move |_| editor.set(Some(Editor::Routine(routine)))
            >
                <span class="rtn-head">
                    <b data-testid="routine-name">
                        {move || {
                            let n = name.get();
                            if n.trim().is_empty() { "（名前なし）".to_string() } else { n }
                        }}
                    </b>
                    <i class="muted" data-testid="routine-count">
                        {move || format!("{} 種目", open_count.get())}
                    </i>
                </span>
                // ★ 名前はアーカイブ済みも出す。アーカイブは可逆なので、ここで隠すと
                //   「入れたはずの種目が無い」に見える。件数と食い違う分は下の注記で説明する
                <span class="rtn-names muted" data-testid="routine-names">
                    {move || names.get().join(", ")}
                </span>
                {move || {
                    // ★ 出ない理由は必ず画面に出す。無いと「作ったのに使えない」原因を
                    //   探しようがない（記録タブ側には何も出せない — 行ごと消えるので）
                    if open_count.get() == 0 {
                        return Some(
                            view! {
                                <span class="rtn-warn" data-testid="routine-unusable">
                                    "使える種目がないため記録タブに出ません"
                                </span>
                            }
                                .into_any(),
                        );
                    }
                    let n = hidden.get();
                    (n > 0)
                        .then(|| {
                            view! {
                                <span class="rtn-warn" data-testid="routine-partial">
                                    {format!("アーカイブ済みの {n} 種目は記録タブに出ません")}
                                </span>
                            }
                                .into_any()
                        })
                }}
            </button>
        </section>
    }
}

/// 部位 1 つぶん（ヘッダ + **開いているときだけ**種目一覧）。
#[component]
fn GroupBlock(
    group: GroupId,
    editor: RwSignal<Option<Editor>>,
    open_group: RwSignal<Option<GroupId>>,
) -> impl IntoView {
    let db = use_db();

    let name = Memo::new(move |_| {
        db.with(|d| d.group(group).map(|g| g.name.clone()))
            .unwrap_or_default()
    });
    let color = Memo::new(move |_| {
        db.with(|d| d.group(group).map(|g| g.color.clone()))
            .unwrap_or_default()
    });
    // 閉じていても件数はヘッダに出すので、`open` の外で購読する
    let ex_ids = Memo::new(move |_| db.with(|d| ordered_exercise_ids(d, group)));

    // ★ `Memo` にする。`open_group.get() == Some(group)` を生クロージャで書くと、
    //   どの部位を開いても部位の数だけ再評価が走る
    let open = Memo::new(move |_| open_group.get() == Some(group));

    view! {
        <section class="card grp" id=grp_dom_id(group) data-testid="group-item">
            <header class="card-head">
                // ★ ヘッダ幅のほぼ全部を占める 1 個のボタン。押し損ねようがない大きさにする。
                //   シェブロンを左端に置くのは、開閉状態が縦一列でスキャンでき、右端を
                //   「操作（鉛筆）」に空けられるから。左 = 状態 / 右 = アクションの分離。
                // ★ <details>/<summary> を使わないのは、<summary> が interactive content で
                //   中に鉛筆ボタンを置けないため（不正 HTML で、iOS Safari では子ボタンの
                //   クリックが summary の既定動作にも届く）。help.rs の InstallBanner が
                //   同じ理由で「箱は <div>、ボタンは兄弟」を採っているのに揃える
                <button
                    class="grp-toggle"
                    data-testid="group-toggle"
                    // ★ シェブロンの回転はこの属性を CSS フックにしている。
                    //   見た目が a11y 属性に依存するので、属性が黙って落ちない
                    aria-expanded=move || if open.get() { "true" } else { "false" }
                    on:click=move |_| {
                        open_group.update(|o| *o = (*o != Some(group)).then_some(group));
                    }
                >
                    // 開いた状態は CSS で 90 度回す（chevron-down を別に持たない）
                    {icon(icon::CHEVRON_RIGHT)}
                    <span class="dot" style=move || format!("--dot:{}", color.get())></span>
                    <span class="grp-name" data-testid="group-name">{move || name.get()}</span>
                    <span class="grp-count muted" data-testid="group-count">
                        {move || format!("{} 種目", ex_ids.get().len())}
                    </span>
                </button>
                // 名前だけでは何のボタンか読めないので aria-label に部位名を入れる
                <button
                    class="icon-btn grp-edit"
                    aria-label=move || format!("{} の名前と色を編集", name.get())
                    data-testid="group-edit"
                    on:click=move |_| editor.set(Some(Editor::Group(group)))
                >
                    {icon(icon::PENCIL)}
                </button>
            </header>

            {move || {
                open.get()
                    .then(|| {
                        view! {
                            <For
                                each=move || ex_ids.get()
                                key=|id| *id
                                children=move |id| view! { <ExerciseRow ex=id editor=editor /> }
                            />

                            <div class="grp-foot">
                                <button
                                    class="link-btn"
                                    data-testid="settings-add-exercise"
                                    on:click=move |_| editor.set(Some(Editor::NewExercise(group)))
                                >
                                    "＋ 種目を追加"
                                </button>
                            </div>
                        }
                    })
            }}
        </section>
    }
}

#[component]
fn ExerciseRow(ex: ExerciseId, editor: RwSignal<Option<Editor>>) -> impl IntoView {
    let db = use_db();

    let name = Memo::new(move |_| {
        db.with(|d| d.exercise(ex).map(|e| e.name.clone()))
            .unwrap_or_default()
    });

    view! {
        // 並び替えの矢印が無くなったので、行は名前ボタン 1 つだけ（`.ex-name` が幅を占める）
        <div class="ex-row" data-testid="exercise-item">
            <button
                class="link-btn ex-name"
                data-testid="exercise-name"
                on:click=move |_| editor.set(Some(Editor::Exercise(ex)))
            >
                {move || name.get()}
            </button>
        </div>
    }
}

/// アーカイブ済み種目。**推移タブからは参照できる**ので、ここは「戻す」だけを出す。
#[component]
fn ArchivedSection(ids: Vec<ExerciseId>, open_group: RwSignal<Option<GroupId>>) -> impl IntoView {
    let db = use_db();
    let count = ids.len();

    view! {
        <section class="archived" data-testid="archived-section">
            <h2 class="archived-head">
                "アーカイブ済み "
                <span class="muted" data-testid="archived-count">{format!("{count} 件")}</span>
            </h2>
            {ids
                .into_iter()
                .map(|ex| {
                    let label = db
                        .with_untracked(|d| {
                            d.exercise(ex)
                                .map(|e| {
                                    let group = d
                                        .group(e.group_id)
                                        .map(|g| g.name.clone())
                                        .unwrap_or_else(|| "(部位なし)".to_string());
                                    format!("{} · {}", e.name, group)
                                })
                        })
                        .unwrap_or_default();
                    view! {
                        <div class="archived-row" data-testid="archived-item">
                            <span>{label}</span>
                            <button
                                class="link-btn"
                                data-testid="unarchive-exercise"
                                on:click=move |_| {
                                    // ★ 戻した先の部位を開く。閉じたままだとアーカイブ行が
                                    //   消えるだけで、種目がどこへ戻ったのか分からない。
                                    // ★ 部位が**実在するときだけ**開く。`core::migrate` と
                                    //   `merge_db` は宙に浮いた group_id を宙に浮いたまま残す
                                    //   ので（この節の `(部位なし)` 表示はそのためにある）、
                                    //   存在しない ID を渡すと「読んでいた部位が閉じるだけで
                                    //   何も開かない」という最悪の形になる
                                    let gid = db
                                        .with_untracked(|d| {
                                            d.exercise(ex)
                                                .map(|e| e.group_id)
                                                .filter(|g| d.group(*g).is_some())
                                        });
                                    db.update(|d| set_archived(d, ex, false));
                                    if let Some(gid) = gid {
                                        open_group.set(Some(gid));
                                    }
                                }
                            >
                                "戻す"
                            </button>
                        </div>
                    }
                })
                .collect::<Vec<_>>()}
        </section>
    }
}

// ── 編集フォーム ────────────────────────────────────────────────────────────

#[component]
fn GroupEditor(
    id: GroupId,
    editor: RwSignal<Option<Editor>>,
    open_group: RwSignal<Option<GroupId>>,
) -> impl IntoView {
    let db = use_db();
    let kb = use_kb();

    let (name0, color0) = db
        .with_untracked(|d| d.group(id).map(|g| (g.name.clone(), g.color.clone())))
        .unwrap_or_default();
    let name = RwSignal::new(name0.clone());
    let confirming = RwSignal::new(false);

    // ★ 削除ガードは **アーカイブ済み種目も所属種目に数える**。
    //   ここを緩めると group_id が宙に浮き、過去ログの部位帰属（カレンダーのドット色・
    //   部位別グラフ・今日タブの部位チップ）が壊れる。
    let counts = Memo::new(move |_| {
        db.with(|d| {
            let ids = d.exercise_ids_of_group(id);
            let archived = ids
                .iter()
                .filter(|ex| d.exercise(**ex).is_some_and(|e| e.archived))
                .count();
            (ids.len(), archived)
        })
    });

    // 空欄は反映しない（入力欄には残るので、打ち直せば続けられる）
    let commit_name = move || {
        let value = name.get_untracked().trim().to_string();
        if !value.is_empty() {
            db.update(move |d| rename_group(d, id, &value));
        }
    };

    view! {
        <label class="field">
            <span>"名前"</span>
            <input
                class="text-input"
                type="text"
                value=name0
                data-testid="group-rename"
                on:focusin=move |_| kb_focus(kb)
                on:focusout=move |_| kb_blur(kb)
                on:input=move |ev| {
                    name.set(event_target_value(&ev));
                    commit_name();
                }
            />
        </label>

        <label class="field">
            <span>"色"</span>
            <input
                type="color"
                value=color0
                data-testid="group-color"
                on:input=move |ev| {
                    let value = event_target_value(&ev);
                    db.update(move |d| set_group_color(d, id, &value));
                }
            />
        </label>

        <div class="sheet-actions">
            <button
                class="link-btn danger"
                data-testid="delete-group"
                on:click=move |_| confirming.set(true)
            >
                "この部位を削除"
            </button>
        </div>

        {move || {
            if !confirming.get() {
                return None;
            }
            let (total, archived) = counts.get();
            Some(
                if total > 0 {
                    view! {
                        <div class="warn-box">
                            // ★ 全部アーカイブ済みなら文言のほうを変える。ヘッダの
                            //   「N 種目」は非アーカイブしか数えないので、そこが 0 なのに
                            //   「種目が 5 件あるため」と言われると、画面のどこを見ても
                            //   理由が読めない状態になる
                            <p data-testid="delete-blocked">
                                {if archived == total {
                                    format!("アーカイブ済み種目が {total} 件あるため削除できません")
                                } else {
                                    format!("種目が {total} 件あるため削除できません")
                                }}
                            </p>
                            {(archived > 0 && archived < total)
                                .then(|| {
                                    view! {
                                        // 画面に出ていない種目が理由なのは分からないので件数を出す
                                        <p data-testid="delete-blocked-archived">
                                            {format!("うち {archived} 件はアーカイブ済みです")}
                                        </p>
                                    }
                                })}
                            <p class="muted">"先に種目を別の部位へ移してください"</p>
                            <button class="link-btn" on:click=move |_| confirming.set(false)>
                                "閉じる"
                            </button>
                        </div>
                    }
                        .into_any()
                    } else {
                        view! {
                            <div class="warn-box">
                                <p>"この部位を削除します"</p>
                                <div class="sheet-actions">
                                    <button
                                        class="primary"
                                        data-testid="delete-group-confirm"
                                        on:click=move |_| {
                                            db.update(|d| delete_group(d, id));
                                            // 消えた部位を開いたままにしない
                                            if open_group.get_untracked() == Some(id) {
                                                open_group.set(None);
                                            }
                                            editor.set(None);
                                        }
                                    >
                                        "削除する"
                                    </button>
                                    <button class="link-btn" on:click=move |_| confirming.set(false)>
                                        "やめる"
                                    </button>
                                </div>
                            </div>
                        }
                            .into_any()
                    },
            )
        }}
    }
}

#[component]
fn NewGroupEditor(
    editor: RwSignal<Option<Editor>>,
    open_group: RwSignal<Option<GroupId>>,
) -> impl IntoView {
    let db = use_db();
    let kb = use_kb();

    let color0 = db
        .with_untracked(|d| COLOR_CHOICES[d.groups.len() % COLOR_CHOICES.len()])
        .to_string();
    let name = RwSignal::new(String::new());
    let color = RwSignal::new(color0.clone());
    let duplicate = RwSignal::new(false);

    let submit = move |_| {
        let value = name.get_untracked().trim().to_string();
        if value.is_empty() {
            return;
        }
        // 同名を許すとプリセット投入の同名スキップと噛み合わなくなる
        if db.with_untracked(|d| d.groups.iter().any(|g| g.name == value)) {
            duplicate.set(true);
            return;
        }
        let picked = color.get_untracked();
        let id = storage::alloc_id();
        db.update(move |d| add_group(d, id, value, picked));
        // ★ 作った部位を開く。中身が空なので、閉じたままだと「＋ 種目を追加」が見えず
        //   行き止まりに見える
        open_group.set(Some(id));
        editor.set(None);
    };

    view! {
        <label class="field">
            <span>"名前"</span>
            <input
                class="text-input"
                type="text"
                data-testid="new-group-name"
                on:focusin=move |_| kb_focus(kb)
                on:focusout=move |_| kb_blur(kb)
                on:input=move |ev| {
                    name.set(event_target_value(&ev));
                    duplicate.set(false);
                }
            />
        </label>

        <label class="field">
            <span>"色"</span>
            <input
                type="color"
                value=color0
                data-testid="new-group-color"
                on:input=move |ev| color.set(event_target_value(&ev))
            />
        </label>

        {move || {
            duplicate
                .get()
                .then(|| {
                    view! {
                        <div class="warn-box">
                            <p data-testid="duplicate-name">"同じ名前の部位があります"</p>
                        </div>
                    }
                })
        }}

        <div class="sheet-actions">
            <button class="primary" data-testid="new-group-submit" on:click=submit>
                "追加"
            </button>
        </div>
    }
}

#[component]
fn ExerciseEditor(
    id: ExerciseId,
    editor: RwSignal<Option<Editor>>,
    open_group: RwSignal<Option<GroupId>>,
) -> impl IntoView {
    let db = use_db();
    let kb = use_kb();

    let name0 = db
        .with_untracked(|d| d.exercise(id).map(|e| e.name.clone()))
        .unwrap_or_default();
    // 部位の選択肢は untracked で固定する（編集中に一覧が動いてボタンが作り直されるのを防ぐ）
    let groups: Vec<(GroupId, String)> = db.with_untracked(|d| {
        ordered_group_ids(d)
            .into_iter()
            .filter_map(|g| d.group(g).map(|g| (g.id, g.name.clone())))
            .collect()
    });
    let name = RwSignal::new(name0.clone());

    let group = Memo::new(move |_| {
        db.with(|d| d.exercise(id).map(|e| e.group_id))
            .unwrap_or_default()
    });

    let commit_name = move || {
        let value = name.get_untracked().trim().to_string();
        if !value.is_empty() {
            db.update(move |d| rename_exercise(d, id, &value));
        }
    };

    view! {
        <label class="field">
            <span>"名前"</span>
            <input
                class="text-input"
                type="text"
                value=name0
                data-testid="exercise-rename"
                on:focusin=move |_| kb_focus(kb)
                on:focusout=move |_| kb_blur(kb)
                on:input=move |ev| {
                    name.set(event_target_value(&ev));
                    commit_name();
                }
            />
        </label>

        <div class="field">
            <span>"部位"</span>
            <div class="opts" data-testid="exercise-groups">
                {groups
                    .into_iter()
                    .map(|(gid, label)| {
                        opt_button(
                            label,
                            "group-option",
                            move || group.get() == gid,
                            move || {
                                // 同じ部位を押し直したときは set_exercise_group が何もしない
                                // ので、アコーディオンも動かさない
                                let moved = db
                                    .with_untracked(|d| d.exercise(id).map(|e| e.group_id))
                                    != Some(gid);
                                db.update(move |d| set_exercise_group(d, id, gid));
                                // ★ 移動先を開く。閉じたままだと「種目が消えた」ように見える
                                if moved {
                                    open_group.set(Some(gid));
                                }
                            },
                        )
                    })
                    .collect::<Vec<_>>()}
            </div>
        </div>

        <div class="sheet-actions">
            <button
                class="secondary"
                data-testid="archive-exercise"
                on:click=move |_| {
                    db.update(move |d| set_archived(d, id, true));
                    editor.set(None);
                }
            >
                "この種目をアーカイブ"
            </button>
        </div>
        <p class="settings-note muted">
            "アーカイブは記録を消しません。過去のログは残り、「種目を追加」に出なくなります"
        </p>
    }
}

#[component]
fn NewExerciseEditor(
    group: GroupId,
    editor: RwSignal<Option<Editor>>,
    open_group: RwSignal<Option<GroupId>>,
) -> impl IntoView {
    let db = use_db();
    let kb = use_kb();

    let group_name = db
        .with_untracked(|d| d.group(group).map(|g| g.name.clone()))
        .unwrap_or_default();
    let name = RwSignal::new(String::new());
    let duplicate = RwSignal::new(false);

    let submit = move |_| {
        let value = name.get_untracked().trim().to_string();
        if value.is_empty() {
            return;
        }
        // アーカイブ済みも含めて全体で見る（同名があると移行時にプリセットの
        // 固定 ID へ寄せられなくなる — core::pin_presets が「ちょうど 1 件」を要求する）
        if db.with_untracked(|d| d.exercises.iter().any(|e| e.name == value)) {
            duplicate.set(true);
            return;
        }
        let id = storage::alloc_id();
        db.update(move |d| add_exercise(d, id, group, value));
        // 追加した種目が見えるように開けておく（この経路は既に開いているはずだが、
        // 「追加したのに何も起きない」を構造的に起こさないため明示する）
        open_group.set(Some(group));
        editor.set(None);
    };

    view! {
        <p class="settings-note muted">{format!("{group_name} に追加します")}</p>

        <label class="field">
            <span>"名前"</span>
            <input
                class="text-input"
                type="text"
                data-testid="new-exercise-name"
                on:focusin=move |_| kb_focus(kb)
                on:focusout=move |_| kb_blur(kb)
                on:input=move |ev| {
                    name.set(event_target_value(&ev));
                    duplicate.set(false);
                }
            />
        </label>

        {move || {
            duplicate
                .get()
                .then(|| {
                    view! {
                        <div class="warn-box">
                            <p data-testid="duplicate-name">"同じ名前の種目があります"</p>
                        </div>
                    }
                })
        }}

        <div class="sheet-actions">
            <button class="primary" data-testid="new-exercise-submit" on:click=submit>
                "追加"
            </button>
        </div>
    }
}
