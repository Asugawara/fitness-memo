//! 種目タブ。部位グループと種目の管理。
//!
//! 設計上の要点:
//!
//! - **部位の削除ガードはアーカイブ済み種目も所属種目に数える。** ここが緩むと
//!   アーカイブ済み種目の `group_id` が宙に浮き、カレンダーのドット色・部位別グラフ・
//!   今日タブの部位チップが**過去分だけ**壊れる。画面に出ていない種目が理由で削除できない
//!   のは分からないので、文言に件数を出す（「アーカイブ済み種目が 2 件あります」）
//! - **`Kind` はデータから推論せず種目の明示属性。** 変更時は必ず「グラフの単位が
//!   変わります」を挟み、ユーザーの明示操作であることを可視化する（自重ディップスを加重へ
//!   移したときに過去データが遡及的に壊れるのを防ぐための設計）
//! - **アーカイブは論理削除。** 過去ログの `exercise_id` 参照を保つのが目的。アーカイブ済み
//!   種目は今日タブの「種目を追加」シートには出ないが、推移タブの種目セレクタからは参照
//!   できる（過去データが参照不能になるのを防ぐ）
//! - 並び替えは iPhone のタッチ操作を優先して**上下の矢印ボタン**にする（ドラッグ&ドロップは
//!   実装も E2E も難しい）
//! - 編集フォームは一覧の外に置いた 1 枚のシートに集約する。一覧の中に入力欄を置くと、
//!   1 文字打つたびに `Db` が動いて一覧ごと作り直され、編集中の文字列が消える

use leptos::prelude::*;

use crate::core;
use crate::model::{Db, Exercise, ExerciseId, Group, GroupId, Kind};

use super::{kb_blur, kb_focus, use_db, use_kb};

const KINDS: [Kind; 3] = [Kind::Weighted, Kind::Bodyweight, Kind::Duration];

/// 部位を追加するときの既定色。プリセットの 6 色を順に回す。
const COLOR_CHOICES: [&str; 6] = [
    "#e0524a", "#2f7fd1", "#e0912a", "#7a56c9", "#2fa06a", "#6b7280",
];

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Weighted => "加重",
        Kind::Bodyweight => "自重",
        Kind::Duration => "時間",
    }
}

/// "加重（kg·回）"。単位を併記して、`Kind` = グラフの単位であることを見せる。
fn kind_text(kind: Kind) -> String {
    format!("{}（{}）", kind_label(kind), core::unit_of(kind))
}

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

/// `id` を隣と入れ替える。端で動かせなければ `false`。
fn swap_neighbor(list: &mut [u32], id: u32, up: bool) -> bool {
    let Some(i) = list.iter().position(|x| *x == id) else {
        return false;
    };
    let next = if up {
        i.checked_sub(1)
    } else {
        (i + 1 < list.len()).then_some(i + 1)
    };
    let Some(j) = next else {
        return false;
    };
    list.swap(i, j);
    true
}

fn apply_group_order(db: &mut Db, ordered: &[GroupId]) {
    for (i, id) in ordered.iter().enumerate() {
        if let Some(g) = db.groups.iter_mut().find(|g| g.id == *id) {
            g.order = i as u32;
        }
    }
}

/// 部位内の `order` を「非アーカイブ（現在の順）→ アーカイブ済み」で 0.. に振り直す。
///
/// アーカイブ済みを常に末尾へ押しやることで、間に挟まったアーカイブ済み種目のせいで
/// 「↑ を押しても表示順が変わらない」という状態が起きなくなる。
fn renumber_exercises(db: &mut Db, group: GroupId) {
    let mut list: Vec<(bool, u32, ExerciseId)> = db
        .exercises
        .iter()
        .filter(|e| e.group_id == group)
        .map(|e| (e.archived, e.order, e.id))
        .collect();
    list.sort_unstable();
    for (i, (_, _, id)) in list.iter().enumerate() {
        if let Some(e) = db.exercises.iter_mut().find(|e| e.id == *id) {
            e.order = i as u32;
        }
    }
}

fn move_group(db: &mut Db, id: GroupId, up: bool) {
    let mut ordered = ordered_group_ids(db);
    if swap_neighbor(&mut ordered, id, up) {
        apply_group_order(db, &ordered);
    }
}

fn move_exercise(db: &mut Db, id: ExerciseId, up: bool) {
    let Some(group) = db.exercise(id).map(|e| e.group_id) else {
        return;
    };
    let mut ordered = ordered_exercise_ids(db, group);
    if !swap_neighbor(&mut ordered, id, up) {
        return;
    }
    for (i, ex) in ordered.iter().enumerate() {
        if let Some(e) = db.exercises.iter_mut().find(|e| e.id == *ex) {
            e.order = i as u32;
        }
    }
    renumber_exercises(db, group);
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

fn add_group(db: &mut Db, name: String, color: String) {
    let id = db.alloc_id();
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

fn add_exercise(db: &mut Db, group: GroupId, name: String, kind: Kind) {
    let id = db.alloc_id();
    // 末尾に付ける（renumber が非アーカイブを先頭へ詰め直す）
    let order = db.exercises.iter().filter(|e| e.group_id == group).count() as u32;
    db.exercises.push(Exercise {
        id,
        name,
        group_id: group,
        kind,
        order,
        archived: false,
    });
    renumber_exercises(db, group);
}

fn set_exercise_group(db: &mut Db, id: ExerciseId, group: GroupId) {
    let Some(from) = db.exercise(id).map(|e| e.group_id) else {
        return;
    };
    if from == group {
        return;
    }
    let order = db.exercises.iter().filter(|e| e.group_id == group).count() as u32;
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.group_id = group;
        e.order = order;
    }
    renumber_exercises(db, from);
    renumber_exercises(db, group);
}

fn set_exercise_kind(db: &mut Db, id: ExerciseId, kind: Kind) {
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.kind = kind;
    }
}

/// アーカイブ = 論理削除。過去ログの `exercise_id` 参照を保つ。
fn set_archived(db: &mut Db, id: ExerciseId, archived: bool) {
    let Some(group) = db.exercise(id).map(|e| e.group_id) else {
        return;
    };
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.archived = archived;
        if !archived {
            // 戻したものは非アーカイブの末尾へ（renumber が詰め直す）
            e.order = u32::MAX;
        }
    }
    renumber_exercises(db, group);
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
}

/// 選択肢ボタン 1 個。`Kind` と部位のどちらにも使う。
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
pub fn Menu() -> impl IntoView {
    let db = use_db();
    let editor: RwSignal<Option<Editor>> = RwSignal::new(None);

    let group_ids = Memo::new(move |_| db.with(ordered_group_ids));
    let archived = Memo::new(move |_| db.with(archived_ids));

    view! {
        <section class="menu" data-testid="screen-menu">
            // プリセットの投入は初回起動時に storage::load が済ませる。再投入の導線は持たない
            // （改名済みプリセットが別種目として復活する挙動があり、得より害が大きかった）
            <header class="menu-head">
                <h1>"種目"</h1>
            </header>

            <p class="menu-note muted">
                "アーカイブした種目は「種目を追加」に出なくなりますが、過去の記録は残り推移タブから参照できます"
            </p>

            <For
                each=move || group_ids.get()
                key=|id| *id
                children=move |id| view! { <GroupBlock group=id editor=editor /> }
            />

            <div class="add-wrap">
                <button
                    class="secondary"
                    data-testid="menu-add-group"
                    on:click=move |_| editor.set(Some(Editor::NewGroup))
                >
                    "＋ 部位を追加"
                </button>
            </div>

            {move || {
                let ids = archived.get();
                (!ids.is_empty()).then(|| view! { <ArchivedSection ids=ids /> })
            }}

            {move || {
                editor
                    .get()
                    .map(|target| {
                        let (title, body) = match target {
                            Editor::Group(id) => {
                                ("部位を編集", view! { <GroupEditor id=id editor=editor /> }.into_any())
                            }
                            Editor::NewGroup => {
                                ("部位を追加", view! { <NewGroupEditor editor=editor /> }.into_any())
                            }
                            Editor::Exercise(id) => {
                                ("種目を編集", view! { <ExerciseEditor id=id editor=editor /> }.into_any())
                            }
                            Editor::NewExercise(group) => {
                                (
                                    "種目を追加",
                                    view! { <NewExerciseEditor group=group editor=editor /> }
                                        .into_any(),
                                )
                            }
                        };
                        view! {
                            // ★ z-index はインライン。`.sheet` / `.bottom-tabs` はどちらも
                            //   position: fixed; bottom: 0 で z-index が auto なので、DOM 順で
                            //   後に来る <nav class="bottom-tabs"> がシートの下端 56px を覆い、
                            //   シート最下部のボタン（削除・追加）が押せなくなる。
                            //   styles.css に `.sheet { z-index: 20 }` が入れば重複するだけで無害。
                            <div
                                class="sheet-backdrop"
                                style="z-index:19"
                                data-testid="menu-sheet-backdrop"
                                on:click=move |_| editor.set(None)
                            ></div>
                            <div
                                class="sheet"
                                style="z-index:20"
                                role="dialog"
                                aria-label=title
                                data-testid="menu-sheet"
                            >
                                <header class="sheet-head">
                                    <strong>{title}</strong>
                                    <button
                                        class="link-btn"
                                        data-testid="menu-sheet-close"
                                        on:click=move |_| editor.set(None)
                                    >
                                        "閉じる"
                                    </button>
                                </header>
                                <div class="sheet-body">{body}</div>
                            </div>
                        }
                    })
            }}
        </section>
    }
}

/// 部位 1 つぶん（ヘッダ + その部位の種目一覧）。
#[component]
fn GroupBlock(group: GroupId, editor: RwSignal<Option<Editor>>) -> impl IntoView {
    let db = use_db();

    let name = Memo::new(move |_| {
        db.with(|d| d.group(group).map(|g| g.name.clone()))
            .unwrap_or_default()
    });
    let color = Memo::new(move |_| {
        db.with(|d| d.group(group).map(|g| g.color.clone()))
            .unwrap_or_default()
    });
    let ex_ids = Memo::new(move |_| db.with(|d| ordered_exercise_ids(d, group)));

    view! {
        <section class="card grp" data-testid="group-item">
            <header class="card-head">
                <span class="dot" style=move || format!("--dot:{}", color.get())></span>
                <button
                    class="link-btn grp-name"
                    data-testid="group-name"
                    on:click=move |_| editor.set(Some(Editor::Group(group)))
                >
                    {move || name.get()}
                </button>
                <span class="grp-count muted" data-testid="group-count">
                    {move || format!("{} 種目", ex_ids.get().len())}
                </span>
                <button
                    class="icon-btn"
                    aria-label="この部位を上へ"
                    data-testid="group-up"
                    on:click=move |_| db.update(|d| move_group(d, group, true))
                >
                    "↑"
                </button>
                <button
                    class="icon-btn"
                    aria-label="この部位を下へ"
                    data-testid="group-down"
                    on:click=move |_| db.update(|d| move_group(d, group, false))
                >
                    "↓"
                </button>
            </header>

            <For
                each=move || ex_ids.get()
                key=|id| *id
                children=move |id| view! { <ExerciseRow ex=id editor=editor /> }
            />

            <div class="grp-foot">
                <button
                    class="link-btn"
                    data-testid="menu-add-exercise"
                    on:click=move |_| editor.set(Some(Editor::NewExercise(group)))
                >
                    "＋ 種目を追加"
                </button>
            </div>
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
    let kind = Memo::new(move |_| {
        db.with(|d| d.exercise(ex).map(|e| e.kind))
            .unwrap_or(Kind::Weighted)
    });

    view! {
        <div class="ex-row" data-testid="exercise-item">
            <button
                class="link-btn ex-name"
                data-testid="exercise-name"
                on:click=move |_| editor.set(Some(Editor::Exercise(ex)))
            >
                {move || name.get()}
            </button>
            <span class="ex-kind muted" data-testid="exercise-kind">
                {move || kind_label(kind.get())}
            </span>
            <button
                class="icon-btn"
                aria-label="この種目を上へ"
                data-testid="exercise-up"
                on:click=move |_| db.update(|d| move_exercise(d, ex, true))
            >
                "↑"
            </button>
            <button
                class="icon-btn"
                aria-label="この種目を下へ"
                data-testid="exercise-down"
                on:click=move |_| db.update(|d| move_exercise(d, ex, false))
            >
                "↓"
            </button>
        </div>
    }
}

/// アーカイブ済み種目。**推移タブからは参照できる**ので、ここは「戻す」だけを出す。
#[component]
fn ArchivedSection(ids: Vec<ExerciseId>) -> impl IntoView {
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
                                    format!("{} · {} · {}", e.name, group, kind_label(e.kind))
                                })
                        })
                        .unwrap_or_default();
                    view! {
                        <div class="archived-row" data-testid="archived-item">
                            <span>{label}</span>
                            <button
                                class="link-btn"
                                data-testid="unarchive-exercise"
                                on:click=move |_| db.update(|d| set_archived(d, ex, false))
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
fn GroupEditor(id: GroupId, editor: RwSignal<Option<Editor>>) -> impl IntoView {
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
                            <p data-testid="delete-blocked">
                                {format!("種目が {total} 件あるため削除できません")}
                            </p>
                            {(archived > 0)
                                .then(|| {
                                    view! {
                                        // 画面に出ていない種目が理由なのは分からないので件数を出す
                                        <p data-testid="delete-blocked-archived">
                                            {format!("アーカイブ済み種目が {archived} 件あります")}
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
fn NewGroupEditor(editor: RwSignal<Option<Editor>>) -> impl IntoView {
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
        db.update(move |d| add_group(d, value, picked));
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
fn ExerciseEditor(id: ExerciseId, editor: RwSignal<Option<Editor>>) -> impl IntoView {
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
    let pending: RwSignal<Option<Kind>> = RwSignal::new(None);

    let kind = Memo::new(move |_| {
        db.with(|d| d.exercise(id).map(|e| e.kind))
            .unwrap_or(Kind::Weighted)
    });
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

    // ★ Kind はデータから推論せず種目の明示属性なので、変更は必ずユーザーの明示操作にする。
    //   タップした時点では確定せず「グラフの単位が変わります」を挟む。
    let shown_kind = move || pending.get().unwrap_or_else(|| kind.get());
    let pick_kind = move |picked: Kind| {
        if picked == kind.get_untracked() {
            pending.set(None);
        } else {
            pending.set(Some(picked));
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
                            move || db.update(move |d| set_exercise_group(d, id, gid)),
                        )
                    })
                    .collect::<Vec<_>>()}
            </div>
        </div>

        <div class="field">
            <span>"種類"</span>
            <div class="opts" data-testid="exercise-kinds">
                {KINDS
                    .map(|k| {
                        opt_button(
                            kind_text(k),
                            "kind-option",
                            move || shown_kind() == k,
                            move || pick_kind(k),
                        )
                    })
                    .into_iter()
                    .collect::<Vec<_>>()}
            </div>
        </div>

        {move || {
            pending
                .get()
                .map(|picked| {
                    let from = core::unit_of(kind.get());
                    let to = core::unit_of(picked);
                    view! {
                        <div class="warn-box">
                            <p data-testid="kind-warning">
                                {format!(
                                    "グラフの単位が変わります（{from} → {to}）。過去の記録も新しい単位で計算し直されます",
                                )}
                            </p>
                            <div class="sheet-actions">
                                <button
                                    class="primary"
                                    data-testid="kind-confirm"
                                    on:click=move |_| {
                                        db.update(move |d| set_exercise_kind(d, id, picked));
                                        pending.set(None);
                                    }
                                >
                                    {format!("{} に変更する", kind_label(picked))}
                                </button>
                                <button
                                    class="link-btn"
                                    data-testid="kind-cancel"
                                    on:click=move |_| pending.set(None)
                                >
                                    "やめる"
                                </button>
                            </div>
                        </div>
                    }
                })
        }}

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
        <p class="menu-note muted">
            "アーカイブは記録を消しません。過去のログは残り、「種目を追加」に出なくなります"
        </p>
    }
}

#[component]
fn NewExerciseEditor(group: GroupId, editor: RwSignal<Option<Editor>>) -> impl IntoView {
    let db = use_db();
    let kb = use_kb();

    let group_name = db
        .with_untracked(|d| d.group(group).map(|g| g.name.clone()))
        .unwrap_or_default();
    let name = RwSignal::new(String::new());
    let kind = RwSignal::new(Kind::Weighted);
    let duplicate = RwSignal::new(false);

    let submit = move |_| {
        let value = name.get_untracked().trim().to_string();
        if value.is_empty() {
            return;
        }
        // アーカイブ済みも含めて全体で見る（presets::seed の同名スキップと同じ規則）
        if db.with_untracked(|d| d.exercises.iter().any(|e| e.name == value)) {
            duplicate.set(true);
            return;
        }
        let picked = kind.get_untracked();
        db.update(move |d| add_exercise(d, group, value, picked));
        editor.set(None);
    };

    view! {
        <p class="menu-note muted">{format!("{group_name} に追加します")}</p>

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

        <div class="field">
            <span>"種類"</span>
            <div class="opts" data-testid="exercise-kinds">
                {KINDS
                    .map(|k| {
                        opt_button(
                            kind_text(k),
                            "kind-option",
                            move || kind.get() == k,
                            move || kind.set(k),
                        )
                    })
                    .into_iter()
                    .collect::<Vec<_>>()}
            </div>
        </div>

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
