//! 並び替えの幾何。**ターゲット非依存なので `cargo test` で検証できる。**
//!
//! ドラッグ中は signal の `Vec` を入れ替えず、`transform` だけで見せて指を離した 1 回で
//! 確定する（adr/ux/drag-to-reorder-in-record-tab.md）。したがって
//! 「掴んだ瞬間に測った箱」＋「指の移動量」→「挿入先」がこのモジュールの全部になる。
//!
//! [グラフの座標計算をテストできるモジュールに切り出す](../adr/architecture/chart-layout-as-a-testable-module.md)
//! と同じ分担で、`core` が `Db` を、`chart_layout` がグラフの寸法を、ここが並びの寸法を知る。
//! **`Db` も leptos も web-sys も知らない。**
//!
//! ## 座標は document 基準
//!
//! [`Slot::top`] は `getBoundingClientRect().top + scrollY`。viewport 基準にしないのは、
//! iOS が慣性スクロール中の `pointerdown` で `pointercancel` を送らないことがあり、
//! ドラッグ中にページが動くと viewport 基準のスナップショットが陳腐化するから。
//! document 基準なら毎 `pointermove` で `scrollY` を足し直すだけで整合が保てる。

/// 画面端の自動スクロールの帯の厚み。[`edge_scroll_step`] の立ち上がりの分母。
///
/// 呼び側は「帯の内側の縁」を渡す（上は `EDGE_BAND`、下は
/// `innerHeight - タブバー - EDGE_BAND`）。72px は 44px のタップ標的より一回り大きく、
/// 指がそこに入ったのが偶然ではないと言える幅。
pub const EDGE_BAND: f64 = 72.0;

/// 並びの 1 要素が占める箱。**document 座標**（`rect.top + scrollY`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub top: f64,
    pub height: f64,
}

impl Slot {
    fn bottom(&self) -> f64 {
        self.top + self.height
    }

    /// 入れ替えの閾値。**相手の箱の中心。**
    fn mid(&self) -> f64 {
        self.top + self.height / 2.0
    }
}

/// 掴んだ瞬間に測った並び全体。**ドラッグ中は測り直さない。**
#[derive(Clone, Debug, PartialEq)]
pub struct Slots(Vec<Slot>);

impl Slots {
    pub fn new(slots: Vec<Slot>) -> Self {
        Self(slots)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `from` を掴んで指を `dy` 動かしたときの挿入先。
    ///
    /// **規則は「掴んだ箱の進行方向の辺が、相手の箱の中心を越えたら 1 つ進む」。**
    ///
    /// ★ 中心 vs 中心で比べてはいけない。閾値が `(h_from + h_k) / 2` になって
    /// **掴んだ側の高さに依存する**ので、400px のカードを動かすのに常に 200px 超の
    /// 指移動が要ることになる。辺 vs 中心なら閾値は `h_k / 2 + 隙間` ＝ **相手の高さ
    /// だけで決まり**、「相手が半分隠れたら入れ替わる」と目で読める。
    ///
    /// 箱が重ならず文書順に並ぶなら閾値は `k` について狭義単調増加なので、返り値は
    /// `dy` について単調になる。**だからヒステリシスが要らない**（要るようになったら
    /// `prev_to` を引数に足した別関数にすること。ここは純関数のまま保つ）。
    ///
    /// `dy` が `NaN` のときはどの比較も偽になるので `from` を返す。
    pub fn drop_index(&self, from: usize, dy: f64) -> usize {
        let Some(g) = self.0.get(from) else {
            return from;
        };
        if dy > 0.0 {
            let edge = g.bottom() + dy;
            let mut to = from;
            for (k, s) in self.0.iter().enumerate().skip(from + 1) {
                if edge < s.mid() {
                    break;
                }
                to = k;
            }
            to
        } else if dy < 0.0 {
            let edge = g.top + dy;
            for (k, s) in self.0.iter().enumerate().take(from) {
                if edge <= s.mid() {
                    return k;
                }
            }
            from
        } else {
            from
        }
    }

    /// 掴んだ要素が空ける縦幅（要素の隙間込み）。押しのけ量はこの 1 つで足りる。
    ///
    /// 掴んだ要素を抜いて挿し直すと、間に挟まれた要素は**自分の高さに関係なく一律
    /// この値だけ**動く（間の要素の前で増減した量がちょうどこれになるため）。
    /// 高さがバラバラでも押しのけ量が 1 つの値で済むのはこのため。
    ///
    /// ★ `.card { margin-bottom: 12px }` のような隙間を**コードに書かないため**の式。
    /// 実測した箱の差から引くので、CSS 側で余白を変えても追随する。末尾の要素だけは
    /// 次の兄弟が無いので「下端の差」＝ `直前との隙間 + 自分の高さ` で代用する。
    ///
    /// 隙間が要素ごとに違う並びでは近似になる。`.set-row`（隙間 0）と `.card`
    /// （一律 12px）はどちらも厳密に一致する。
    pub fn pitch(&self, from: usize) -> f64 {
        if self.0.len() < 2 || from >= self.0.len() {
            return 0.0;
        }
        match self.0.get(from + 1) {
            Some(next) => next.top - self.0[from].top,
            // 末尾。`from - 1` は len >= 2 かつ from == len-1 なので必ず存在する
            None => self.0[from].bottom() - self.0[from - 1].bottom(),
        }
    }

    /// 掴んでいない要素 `i` に当てる `translateY`。動かないときは `None`。
    ///
    /// `None` を返すとビュー側がインラインスタイルごと消す（`transform: none` の
    /// 残骸を残さない）ので、静止時の DOM に 1 文字も足さずに済む。
    pub fn offset(&self, from: usize, to: usize, i: usize) -> Option<f64> {
        if i == from || from == to {
            return None;
        }
        let pitch = self.pitch(from);
        let moved = if to > from {
            (i > from && i <= to).then_some(-pitch)
        } else {
            (i >= to && i < from).then_some(pitch)
        };
        moved.filter(|o| *o != 0.0)
    }

    /// 掴んだ要素に当てる `translateY`。並びの外へ飛ばないよう clamp する。
    ///
    /// ★ `dy` が有限であることを**ここで**確かめる。`f64::clamp` は `NaN` をそのまま
    /// 返すので、通すと `translateY(NaNpx)` という黙って無視されるスタイルになり、
    /// 「掴んでいるのに指について来ない」という原因の分からない壊れ方をする。
    pub fn lift(&self, from: usize, dy: f64) -> f64 {
        if !dy.is_finite() {
            return 0.0;
        }
        let (Some(g), Some(first), Some(last)) = (self.0.get(from), self.0.first(), self.0.last())
        else {
            return 0.0;
        };
        // 掴んだ要素は並びの中にあるので min <= 0 <= max が必ず成り立つ
        let min = first.top - g.top;
        let max = last.bottom() - g.bottom();
        dy.clamp(min, max)
    }
}

/// `v[from]` を抜いて `to` へ挿す。**`to` は挿し込んだ後の列における添字**
/// （＝呼んだ後に `v[to]` が元の `v[from]` になる）。
///
/// 範囲外の `from` は何もしない。`to` は末尾に丸める。**panic しない。**
///
/// ★ `views` に置かないこと。`remove` してから `insert` する式は添字が 1 ずれる古典的な
/// 事故の場所で、`views` は wasm32 の cfg gate の内側にあるので `cargo test` が
/// 一度も触れない（[`crate::core::short_elapsed`] の doc にある `ms / 86_400_000` の
/// 前例がまさにこれ）。
pub fn move_item<T>(v: &mut [T], from: usize, to: usize) {
    if from >= v.len() {
        return;
    }
    let to = to.min(v.len() - 1);
    if from == to {
        return;
    }
    // `Vec::remove` + `Vec::insert` と同じ結果を、所有権を動かさず回転で作る。
    // `&mut [T]` で足りるので呼び側が `Vec` である必要が無くなる
    if from < to {
        v[from..=to].rotate_left(1);
    } else {
        v[to..=from].rotate_right(1);
    }
}

/// 並び替えの途中で、模型上 `i` 番目の要素が**画面で何番目に見えているか**。
///
/// ドラッグ中は `Vec` を入れ替えないので、模型の添字と見えている位置がずれる。
/// セット行の番号（`.set-no`）はこれを通してから描く。通さないと、掴んだ行が
/// 2 番目に見えているのに「1」と書いてあるという状態になり、**その番号が順番だという
/// 前提そのもの**（この機能が番号をハンドルにした理由）が指を離すまで嘘になる。
/// これを通せば、掴んだ番号が中点を越えた瞬間に変わり、落ちる先が先に読める。
pub fn visual_index(from: usize, to: usize, i: usize) -> usize {
    if i == from {
        to
    } else if to > from && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

/// `from` の 1 つ隣。**端では `from` のまま**なので、呼び側は `from == to` の分岐 1 本で
/// 「端で何も起きない」を扱える。
///
/// ドラッグの代わりのキーボード操作（`Alt` + ↑↓）が使う。
pub fn neighbor(from: usize, up: bool, len: usize) -> usize {
    if up {
        from.saturating_sub(1)
    } else {
        (from + 1).min(len.saturating_sub(1))
    }
}

/// 画面端の自動スクロール量（px/frame）。**帯の外では 0。**
///
/// `client_y` は viewport 座標。`top` / `bottom` は帯の**内側の縁**で、そこから
/// [`EDGE_BAND`] 進むまでに 0 → `max_step` へ線形に立ち上がり、以降は飽和する。
/// 上へ動かすときは負を返す。
///
/// 指が止まっていてもスクロールし続ける必要があるので、呼び側は `pointermove` では
/// なく `requestAnimationFrame` のループから呼ぶこと。
pub fn edge_scroll_step(client_y: f64, top: f64, bottom: f64, max_step: f64) -> f64 {
    if !client_y.is_finite() {
        return 0.0;
    }
    if client_y < top {
        -max_step * ((top - client_y) / EDGE_BAND).clamp(0.0, 1.0)
    } else if client_y > bottom {
        max_step * ((client_y - bottom) / EDGE_BAND).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 高さ `h` の箱を隙間 `gap` で `n` 個並べる。
    fn even(n: usize, h: f64, gap: f64) -> Slots {
        Slots::new(
            (0..n)
                .map(|i| Slot {
                    top: i as f64 * (h + gap),
                    height: h,
                })
                .collect(),
        )
    }

    /// 高さがバラバラな並び（隙間 12px）。`.card` の実測に近い形。
    fn ragged(heights: &[f64]) -> Slots {
        let mut top = 0.0;
        let mut v = Vec::new();
        for h in heights {
            v.push(Slot { top, height: *h });
            top += h + 12.0;
        }
        Slots::new(v)
    }

    // ── drop_index ──────────────────────────────────────────────────────────

    #[test]
    fn an_empty_or_single_list_never_moves() {
        assert_eq!(Slots::new(Vec::new()).drop_index(0, 999.0), 0);
        let one = even(1, 50.0, 0.0);
        assert_eq!(one.drop_index(0, 999.0), 0, "下へ振り切っても動かない");
        assert_eq!(one.drop_index(0, -999.0), 0, "上へ振り切っても動かない");
    }

    #[test]
    fn a_still_finger_always_lands_where_it_started() {
        // ★ タップで並びが変わらないことの根拠。ここが from を返さなくなったら、
        //   見出しに触っただけで種目の順が入れ替わる
        let s = ragged(&[200.0, 380.0, 150.0, 260.0]);
        for from in 0..s.len() {
            assert_eq!(s.drop_index(from, 0.0), from, "from = {from}");
        }
    }

    #[test]
    fn a_finger_tremor_of_a_few_pixels_lands_where_it_started() {
        let s = even(5, 50.0, 0.0);
        for dy in [-3.0, -1.0, 1.0, 3.0] {
            assert_eq!(s.drop_index(2, dy), 2, "dy = {dy}");
        }
    }

    #[test]
    fn moving_down_advances_once_the_next_box_is_half_covered() {
        let s = even(4, 50.0, 0.0);
        assert_eq!(s.drop_index(0, 24.0), 0, "隣の中心の手前では動かない");
        assert_eq!(
            s.drop_index(0, 25.0),
            1,
            "隣の高さの半分でちょうど 1 つ進む"
        );
        assert_eq!(s.drop_index(0, 74.0), 1);
        assert_eq!(s.drop_index(0, 75.0), 2);
    }

    #[test]
    fn moving_up_advances_once_the_previous_box_is_half_covered() {
        let s = even(4, 50.0, 0.0);
        assert_eq!(s.drop_index(3, -24.0), 3);
        assert_eq!(s.drop_index(3, -25.0), 2);
        assert_eq!(s.drop_index(3, -75.0), 1);
    }

    #[test]
    fn the_first_item_cannot_go_up_and_the_last_cannot_go_down() {
        let s = even(4, 50.0, 0.0);
        assert_eq!(s.drop_index(0, -9999.0), 0);
        assert_eq!(s.drop_index(3, 9999.0), 3);
    }

    #[test]
    fn the_threshold_is_the_target_height_not_the_dragged_height() {
        // ★ この式の肝。掴んだ箱が 380px でも、隣の 150px の箱を越えるのに要る
        //   指の移動は 150/2 + 隙間 12 = 87px で済む。中心 vs 中心にすると
        //   (380 + 150) / 2 = 265px 要ることになる
        let s = ragged(&[200.0, 380.0, 150.0, 260.0]);
        assert_eq!(s.drop_index(1, 86.0), 1);
        assert_eq!(s.drop_index(1, 87.0), 2);

        // 逆向き（低い箱で高い箱を越える）も、閾値は相手の高さだけで決まる
        let up = ragged(&[380.0, 150.0]);
        assert_eq!(up.drop_index(1, -(380.0 / 2.0 + 12.0) + 1.0), 1);
        assert_eq!(up.drop_index(1, -(380.0 / 2.0 + 12.0)), 0);
    }

    #[test]
    fn a_zero_height_box_is_decided_without_panicking() {
        let s = Slots::new(vec![
            Slot {
                top: 0.0,
                height: 0.0,
            },
            Slot {
                top: 0.0,
                height: 50.0,
            },
            Slot {
                top: 50.0,
                height: 0.0,
            },
        ]);
        assert_eq!(s.drop_index(1, 0.0), 1);
        assert_eq!(s.drop_index(1, 60.0), 2);
        assert_eq!(s.drop_index(1, -1.0), 0);
    }

    #[test]
    fn an_extreme_or_non_numeric_dy_saturates_instead_of_wrapping() {
        let s = even(5, 50.0, 0.0);
        assert_eq!(s.drop_index(2, f64::MAX), 4);
        assert_eq!(s.drop_index(2, -f64::MAX), 0);
        assert_eq!(s.drop_index(2, f64::INFINITY), 4);
        assert_eq!(s.drop_index(2, f64::NAN), 2, "NaN は動かさない");
    }

    #[test]
    fn an_out_of_range_from_is_returned_untouched() {
        let s = even(3, 50.0, 0.0);
        assert_eq!(s.drop_index(9, 100.0), 9);
    }

    // ── pitch / offset ──────────────────────────────────────────────────────

    #[test]
    fn pitch_is_the_space_the_dragged_box_frees_including_the_gap() {
        let s = ragged(&[200.0, 380.0, 150.0]);
        assert_eq!(s.pitch(0), 212.0, "自分の高さ + 下の隙間");
        assert_eq!(s.pitch(1), 392.0);
        assert_eq!(s.pitch(2), 162.0, "末尾は上の隙間 + 自分の高さで代用する");
        assert_eq!(
            even(1, 50.0, 0.0).pitch(0),
            0.0,
            "1 個なら押しのけようがない"
        );
    }

    #[test]
    fn only_the_boxes_between_from_and_to_are_pushed_aside() {
        let s = even(5, 50.0, 0.0);
        // 0 を 2 へ: 1 と 2 が上へ 1 スロットぶん、0 と 3, 4 は動かない
        assert_eq!(s.offset(0, 2, 0), None, "掴んでいる要素はここでは扱わない");
        assert_eq!(s.offset(0, 2, 1), Some(-50.0));
        assert_eq!(s.offset(0, 2, 2), Some(-50.0));
        assert_eq!(s.offset(0, 2, 3), None);
        // 4 を 1 へ: 1, 2, 3 が下へ
        assert_eq!(s.offset(4, 1, 0), None);
        assert_eq!(s.offset(4, 1, 1), Some(50.0));
        assert_eq!(s.offset(4, 1, 3), Some(50.0));
        assert_eq!(s.offset(4, 1, 4), None);
    }

    #[test]
    fn nothing_is_pushed_aside_when_the_drop_lands_where_it_started() {
        let s = even(4, 50.0, 0.0);
        for i in 0..s.len() {
            assert_eq!(s.offset(2, 2, i), None, "i = {i}");
        }
    }

    #[test]
    fn boxes_of_different_heights_are_all_pushed_by_the_same_pitch() {
        // ★ 押しのけ量が 1 つの値で済むことの主張。ここが崩れると、行ごとに
        //   別の translateY を出す必要があり、隙間の見積りがコードに漏れる
        let s = ragged(&[200.0, 380.0, 150.0, 260.0]);
        assert_eq!(s.offset(0, 3, 1), s.offset(0, 3, 2));
        assert_eq!(s.offset(0, 3, 2), s.offset(0, 3, 3));
        assert_eq!(s.offset(0, 3, 1), Some(-212.0));
    }

    // ── lift ────────────────────────────────────────────────────────────────

    #[test]
    fn lift_never_leaves_the_list() {
        let s = even(4, 50.0, 0.0);
        assert_eq!(s.lift(0, -9999.0), 0.0, "先頭は上へ出られない");
        assert_eq!(s.lift(3, 9999.0), 0.0, "末尾は下へ出られない");
        assert_eq!(s.lift(0, 9999.0), 150.0, "先頭は末尾の下端まで");
        assert_eq!(s.lift(2, 10.0), 10.0, "範囲内はそのまま");
    }

    #[test]
    fn lift_never_produces_a_non_finite_offset() {
        // ★ f64::clamp は NaN をそのまま返す。通すと translateY(NaNpx) になり、
        //   スタイルが黙って無効化されて「掴んだのに指について来ない」になる
        let s = even(4, 50.0, 0.0);
        for dy in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(s.lift(1, dy).is_finite(), "dy = {dy}");
        }
        assert_eq!(s.lift(9, 10.0), 0.0, "範囲外の from も有限");
    }

    #[test]
    fn clamping_the_lift_never_changes_where_it_drops() {
        // clamp の上限は最大の閾値より末尾の高さの半分だけ大きいので、
        // 「見た目は端で止まっているのに挿入先だけ先へ進む」は起きない
        let s = ragged(&[200.0, 380.0, 150.0, 260.0]);
        for from in 0..s.len() {
            for dy in [-9999.0, -300.0, -87.0, 0.0, 87.0, 300.0, 9999.0] {
                assert_eq!(
                    s.drop_index(from, dy),
                    s.drop_index(from, s.lift(from, dy)),
                    "from = {from}, dy = {dy}"
                );
            }
        }
    }

    // ── move_item ───────────────────────────────────────────────────────────

    #[test]
    fn move_item_lands_the_element_on_the_requested_index() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        move_item(&mut v, 0, 2);
        assert_eq!(v, vec!['b', 'c', 'a', 'd']);
        assert_eq!(v[2], 'a', "to は挿し込んだ後の添字");
    }

    #[test]
    fn move_item_works_backwards() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        move_item(&mut v, 3, 0);
        assert_eq!(v, vec!['d', 'a', 'b', 'c']);
    }

    #[test]
    fn move_item_to_the_last_index() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        move_item(&mut v, 0, 3);
        assert_eq!(v, vec!['b', 'c', 'd', 'a']);
    }

    #[test]
    fn move_item_with_the_same_index_is_a_noop() {
        let mut v = vec!['a', 'b', 'c'];
        move_item(&mut v, 1, 1);
        assert_eq!(v, vec!['a', 'b', 'c']);
    }

    #[test]
    fn move_item_ignores_an_out_of_range_from_and_clamps_to() {
        let mut v = vec!['a', 'b', 'c'];
        move_item(&mut v, 9, 0);
        assert_eq!(v, vec!['a', 'b', 'c'], "範囲外の from は何もしない");
        move_item(&mut v, 0, 9);
        assert_eq!(v, vec!['b', 'c', 'a'], "to は末尾へ丸める");
        let mut empty: Vec<char> = Vec::new();
        move_item(&mut empty, 0, 0);
        assert!(empty.is_empty(), "空でも panic しない");
    }

    #[test]
    fn move_item_never_changes_the_multiset() {
        for n in 1..6usize {
            for from in 0..n {
                for to in 0..n {
                    let mut v: Vec<usize> = (0..n).collect();
                    move_item(&mut v, from, to);
                    let mut sorted = v.clone();
                    sorted.sort_unstable();
                    assert_eq!(
                        sorted,
                        (0..n).collect::<Vec<_>>(),
                        "n = {n}, from = {from}, to = {to}"
                    );
                }
            }
        }
    }

    #[test]
    fn move_item_agrees_with_remove_then_insert() {
        for n in 1..6usize {
            for from in 0..n {
                for to in 0..n {
                    let mut got: Vec<usize> = (0..n).collect();
                    move_item(&mut got, from, to);

                    let mut want: Vec<usize> = (0..n).collect();
                    let item = want.remove(from);
                    want.insert(to, item);

                    assert_eq!(got, want, "n = {n}, from = {from}, to = {to}");
                }
            }
        }
    }

    // ── visual_index ────────────────────────────────────────────────────────

    #[test]
    fn visual_index_is_a_permutation_that_matches_move_item() {
        // ★ 画面の並びは「落としたときの並び」と一致していなければならない。
        //   move_item の結果と総当りで突き合わせる
        for n in 1..6usize {
            for from in 0..n {
                for to in 0..n {
                    let mut moved: Vec<usize> = (0..n).collect();
                    move_item(&mut moved, from, to);
                    for (i, model) in (0..n).enumerate() {
                        let seen = visual_index(from, to, model);
                        assert_eq!(
                            moved[seen], model,
                            "n = {n}, from = {from}, to = {to}, i = {i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn visual_index_is_the_identity_while_nothing_has_moved() {
        for i in 0..5 {
            assert_eq!(visual_index(2, 2, i), i);
        }
    }

    // ── neighbor ────────────────────────────────────────────────────────────

    #[test]
    fn neighbor_steps_one_slot_and_stops_at_the_ends() {
        assert_eq!(neighbor(2, true, 5), 1);
        assert_eq!(neighbor(2, false, 5), 3);
        assert_eq!(neighbor(0, true, 5), 0, "先頭は上へ行けない");
        assert_eq!(neighbor(4, false, 5), 4, "末尾は下へ行けない");
        assert_eq!(neighbor(0, true, 1), 0);
        assert_eq!(neighbor(0, false, 1), 0);
        assert_eq!(neighbor(0, false, 0), 0, "空でも panic しない");
    }

    // ── edge_scroll_step ────────────────────────────────────────────────────

    #[test]
    fn the_edge_scroll_is_zero_in_the_middle_of_the_screen() {
        assert_eq!(edge_scroll_step(400.0, 72.0, 700.0, 12.0), 0.0);
        assert_eq!(edge_scroll_step(72.0, 72.0, 700.0, 12.0), 0.0, "縁は帯の外");
        assert_eq!(edge_scroll_step(700.0, 72.0, 700.0, 12.0), 0.0);
    }

    #[test]
    fn the_edge_scroll_ramps_up_inside_the_band_and_saturates() {
        let step = |y| edge_scroll_step(y, 72.0, 700.0, 12.0);
        assert_eq!(step(72.0 - EDGE_BAND / 2.0), -6.0, "帯の半分で半分の速さ");
        assert_eq!(step(72.0 - EDGE_BAND), -12.0);
        assert_eq!(step(-500.0), -12.0, "帯を越えても飽和する");
        assert_eq!(step(700.0 + EDGE_BAND / 2.0), 6.0);
        assert_eq!(step(9999.0), 12.0);
    }

    #[test]
    fn the_edge_scroll_is_zero_for_a_non_numeric_position() {
        assert_eq!(edge_scroll_step(f64::NAN, 72.0, 700.0, 12.0), 0.0);
    }
}
