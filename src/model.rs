//! 永続化されるデータモデル。
//!
//! ID は [`Id`] — 60 bit の乱数で、JSON では 12 文字の base32 文字列になる。
//! 採番は `storage::alloc_id`（wasm 側）または [`IdGen`]（テスト・移行）を通す。
//! プリセットだけは予約領域の固定 ID を持つので採番しない。

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// `Db::schema` の現在値。
///
/// 2 で `Exercise.kind`（加重 / 自重 / 時間）を廃止した。指標は「重量 × 回数、
/// 重量が空なら重量 1」の単一式になり、どの軸で見るかは画面側の設定
/// （[`crate::core::Metric`]）が持つ。
///
/// 3 で ID を連番 `u32` から 60 bit 乱数（[`Id`]）に変え、`next_id` を落とした。
/// 連番のままだと 2 台のデータを混ぜた瞬間に別種目の履歴が入れ替わる。
///
/// ★ フィールドを消す変更は前方互換を壊す（旧版の serde が `missing field` で
/// 拒否する）。schema を上げるときは `storage::KEY` も必ず切ること。
///
/// ★ 逆に **`#[serde(default)]` を持つフィールドの追加では上げない**。旧版は
/// `deny_unknown_fields` を付けていないので未知フィールドを黙って無視し、`migrate` が
/// `Err` を返さないので退避パスにも落ちない。[`SetEntry::note`] / [`ExerciseLog::note`] /
/// [`Db::routines`] がこれで通っている。ここで上げると逆に旧版が
/// `RestoreError::Unsupported` で退避するので、**前方互換を積極的に壊す側になる**。
pub const SCHEMA: u32 = 3;

pub type GroupId = Id<GroupTag>;
pub type ExerciseId = Id<ExerciseTag>;
pub type RoutineId = Id<RoutineTag>;

// ── ID ──────────────────────────────────────────────────────────────────────
//
// 連番 ID は 2 台のデータを混ぜた瞬間に壊れる。同じ種目を登録順だけ変えて登録した
// 2 台では、A の `exercise_id = 2`（ベンチプレス）が B では別の種目を指す。
// `migrate` はそれを正常なデータとして受理するので、気づくのは数か月後に
// グラフを見たときになる。だから ID を乱数にする。

/// Crockford base32。`i` `l` `o` `u` を外してあるので目で読み違えない。
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// JSON に出る ID の文字数。5 bit × 12 = 60 bit をちょうど使い切る。
const ID_LEN: usize = 12;
const ID_BITS: u32 = 60;
const ID_MASK: u64 = (1 << ID_BITS) - 1;

/// プリセットのために予約した領域。[`IdGen`] はここを引かない。
///
/// 予約するのは 1024 個だが乱数は 2^60 から引くので、衝突確率への寄与は 2^-50 で
/// 無視できる。adr/data-model/sequential-ids-no-uuid.md が「固定 ID はユーザー作成の種目と衝突するので採れない」と
/// 諦めた制約は、この予約領域で消える。
pub const RESERVED_MAX: u64 = 1024;

/// 部位 / 種目の識別子。**60 bit の乱数**で、JSON では 12 文字の base32 文字列になる。
///
/// ★ **JSON で数値にしてはいけない。** u64 は 2^53 を超えるので、`e2e/smoke.spec.mjs`
/// の `JSON.parse` → `JSON.stringify` 往復で全 ID が静かに丸められ、参照が壊れる
/// （まさにこの型が潰そうとしているバグと同型）。文字列ならこの経路が構造的に消える。
///
/// `T` は [`GroupTag`] / [`ExerciseTag`] のいずれか。`db.group(exercise_id)` を
/// コンパイルエラーにするためだけに存在し、実行時の表現には現れない。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id<T>(u64, PhantomData<T>);

/// 部位 ID のタグ。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GroupTag;

/// 種目 ID のタグ。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ExerciseTag;

/// トレーニングメニュー ID のタグ。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RoutineTag;

impl<T> Id<T> {
    /// 予約領域の固定 ID を書くための入口。`const` なので `presets.rs` の定数に使える。
    ///
    /// 60 bit に収まらない上位ビットは落とす（`Display` が 12 文字しか出さないので、
    /// 残すと文字列表現との往復が壊れる）。
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits & ID_MASK, PhantomData)
    }

    pub const fn bits(&self) -> u64 {
        self.0
    }

    /// 予約領域の ID か。移行時に「プリセット由来」を見分けるのに使う。
    pub const fn is_reserved(&self) -> bool {
        self.0 < RESERVED_MAX
    }
}

/// `Id(0)` は**どこにも存在しない ID** を表す番兵。
///
/// [`IdGen`] は 0 を返さず（予約領域を避けるため）、プリセットも 0 を使わないので、
/// 実データと衝突しない。「対象が見つからない」を `Option` で持ち回るほどでもない
/// 画面側の一時値に使う。
impl<T> Default for Id<T> {
    fn default() -> Self {
        Self(0, PhantomData)
    }
}

impl<T> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; ID_LEN];
        for (i, slot) in buf.iter_mut().enumerate() {
            let shift = ID_BITS - 5 * (i as u32 + 1);
            *slot = ALPHABET[((self.0 >> shift) & 0x1f) as usize];
        }
        // ALPHABET は ASCII のみなので UTF-8 として必ず妥当
        f.write_str(std::str::from_utf8(&buf).expect("base32 の英数字は常に UTF-8"))
    }
}

/// `Debug` を `Display` に委譲する。`Id(3491...)` より `00000000012j` のほうが
/// テストの失敗メッセージを読める。
impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// [`Id`] のパース失敗。**理由を分けない** — 呼び側（serde と `core::migrate`）は
/// どちらも「読めなかった」以上のことをしないため。
#[derive(Debug, PartialEq, Eq)]
pub struct IdParseError;

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ID は {ID_LEN} 文字の base32 文字列でなければならない")
    }
}

impl std::error::Error for IdParseError {}

impl<T> FromStr for Id<T> {
    type Err = IdParseError;

    /// **厳格にパースする。** 長さちょうど 12、文字集合外は拒否。
    /// 緩めると、version dispatch を素通りした壊れたデータが `Db` に入る。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != ID_LEN {
            return Err(IdParseError);
        }
        let mut bits = 0u64;
        for b in s.bytes() {
            let v = ALPHABET.iter().position(|c| *c == b).ok_or(IdParseError)?;
            bits = (bits << 5) | v as u64;
        }
        Ok(Self(bits, PhantomData))
    }
}

impl<T> Serialize for Id<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de, T> Deserialize<'de> for Id<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // ★ `String` で受けるので、schema 2 以前の数値 ID はここで型エラーになる。
        //   黙って通すと version dispatch を素通りした壊れたデータが生まれる
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// ID 生成器。**乱数源はコンストラクタのシードだけ**なので、この型自体は純関数。
///
/// `core` / `presets` はこれを引数で受け取るので `web-sys` に触れずに済み、ホストの
/// `cargo test` がそのまま動く（[`IdGen::from_seed`] に定数を渡せば ID 列は決定的）。
/// シードを引くのは `storage::crypto_seed` の 1 箇所だけ。
///
/// **`Db` に持たせてはいけない。** エクスポートで PRNG の状態ごと複製され、
/// 2 台が同じ ID 列を生成するようになる。
pub struct IdGen {
    state: u64,
}

impl IdGen {
    pub fn from_seed(seed: u64) -> Self {
        Self { state: seed }
    }

    /// SplitMix64。依存クレートを増やさずに全周期 2^64 と十分な統計品質が得られる。
    ///
    /// `next` という名前にしないのは、`Iterator` を実装していないのに紛らわしく、
    /// clippy の `should_implement_trait` にも引っかかるため。
    pub fn alloc<T>(&mut self) -> Id<T> {
        loop {
            self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            let bits = (z ^ (z >> 31)) & ID_MASK;
            // 予約領域はプリセットのものなので引き直す
            if bits >= RESERVED_MAX {
                return Id(bits, PhantomData);
            }
        }
    }
}

/// 部位（胸 / 背中 / …）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub color: String,
    pub order: u32,
}

/// 種目。
///
/// ★ 指標の種類（旧 `Kind`: 加重 / 自重 / 時間）は**持たない**。種目名を見れば
/// 懸垂が自重でプランクが時間だと分かるので、ユーザーに選ばせる意味が無かった。
/// 指標は [`crate::core::set_volume`] の単一式に統一され、どの軸で見るかは
/// [`crate::core::Metric`]（画面の表示設定）が決める。
///
/// schema 1 の JSON に残っている `"kind"` は serde が未知フィールドとして無視する
/// （`deny_unknown_fields` を付けていないのはこのため）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Exercise {
    pub id: ExerciseId,
    pub name: String,
    pub group_id: GroupId,
    pub order: u32,
    #[serde(default)]
    pub archived: bool,
}

/// 名前付きの種目リスト。**UI では「トレーニングメニュー」**。
///
/// ★ 型名を `Menu` にしない。このリポジトリでは「メニュー」が既に 2 つの意味で
/// 使われている（設定タブの [`crate::views::settings`] と、過去の日の種目構成を指す
/// [`crate::core::MenuCandidate`] / `recent_menus`）。3 つ目を同じ語に載せると、
/// どの「メニュー」の話をしているのかコードから読めなくなる。
///
/// ★ **セットの重量・回数を持たない。** 数値は展開時に種目ごとの
/// [`crate::core::last_log_before`] から引く。目標値を持たせると、記録が伸びても
/// 目標が古いまま腐り、**表示された数値が前回の実績と食い違う** —
/// 「前回を見て同じかそれ以上をやる」という唯一のループが壊れる。
///
/// ★ `order` も持たない。`Vec` の順が表示順（[`Session::logs`] と同じ）。`Group` /
/// `Exercise` が `order` を持つのは、全部位ぶんが 1 本の `Vec` に混ざっていて
/// 部位内の順序を `Vec` 位置で表せないため。こちらは平坦な 1 本なのでその制約が無い。
///
/// ★ `archived` も持たない。**メニューは物理削除でよい** — 種目と違って、これを
/// 参照する記録が 1 つも無いので、消しても過去のログは 1 バイトも欠けない。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Routine {
    pub id: RoutineId,
    pub name: String,
    /// 展開したときのカードの並び順。
    ///
    /// 不変条件: `ExerciseId` は重複しない（[`crate::core::migrate`] が正規化する）。
    /// 存在しない種目・アーカイブ済みの種目を指していることは**ある**（他端末の
    /// データを取り込むと起きる）。読み出し側の `core::expandable` が外す。
    pub exercises: Vec<ExerciseId>,
}

impl Routine {
    /// 名前も種目も無い = 保存する価値がない。
    ///
    /// ★ 「種目が無い」ではない。名前だけ打って種目を選ぶ前に閉じた状態を消して
    /// はいけない（[`ExerciseLog::is_empty`] がメモだけのログを残すのと同じ理由）。
    pub fn is_empty(&self) -> bool {
        self.name.trim().is_empty() && self.exercises.is_empty()
    }
}

/// 1 セット。重量 × 回数と、そのセットのメモ。
///
/// ★ **`Copy` は付けられない**（`note: String` を持つ）。外しても壊れた箇所は無かった —
/// 参照する側は全て `&SetEntry` か `Vec<SetEntry>` の move / clone を通っている。
///
/// ★ `PartialEq` にメモが入ったので、**「同じセットか」の判定に `==` を使ってはいけない**。
/// 重量と回数だけを見る [`crate::core::same_sets`] を通すこと。`==` のままだと
/// 「セットは同じでメモだけ違う」が食い違い扱いになり、取り込みで黙って捨てられる。
///
/// ★ `skip_serializing_if` は容量と互換の両方のため。空メモを毎セット書くと
/// 1 セット ≈ 25 → 35 bytes（+40%）で adr/storage/localstorage-single-key-json.md の
/// 見積りに直接効く。書かなければメモを使っていない利用者の JSON は**今までとバイト単位で
/// 同一**で、`e2e/calendar.spec.mjs` などの `toEqual([{weight, reps}])` が
/// 「保存形式は変わっていない」ことを主張し続ける。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SetEntry {
    pub weight: f32,
    pub reps: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExerciseLog {
    pub exercise_id: ExerciseId,
    pub sets: Vec<SetEntry>,
    /// ★ 当日入力時のみ `Some(epoch ms)`。過去日バックフィルは `None`。
    ///
    /// 「いつトレーニングしたか」の真実源は**日付キーだけ**にする。経過日数は必ず
    /// 日付キーから出すので（adr/data-model/elapsed-in-local-calendar-days.md）`at` を書いても日付が嘘になることはないが、
    /// 過去日に `at = now` を入れると「その日に実施した時刻」として存在しない値が
    /// 残り、同じ暦日の中の時刻表記が捏造される。記録は起きたとおりに持つ。
    ///
    /// ★ メモだけを書いたログには**押さない**。メモを書くのはトレーニングではないので、
    /// セットが 1 本も無いログに時刻が入ると「その日に実施した」証拠を持ってしまう。
    #[serde(default)]
    pub at: Option<i64>,
    /// **その日のその種目**のメモ（adr/data-model/notes-on-logs-and-sets.md）。
    ///
    /// ★ 種目マスタ（[`Exercise`]）には置かない。種目側に置くと毎日同じ注記が出て
    /// 「今日そこがどうだったか」が書けない。[`Session::note`]（その日の体調）とも別で、
    /// こちらは種目に閉じる。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl ExerciseLog {
    /// セットもメモも無い = 保存する価値がない。
    ///
    /// ★ 「セットが無い」ではない。セットを 1 本も入れずに「肩が痛いので今日はやめた」と
    /// 書いた状態を消してはいけない（[`Session::is_empty`] が体重・体調メモだけの日を
    /// 残しているのと同じ理由）。**「トレした日か」の判定はこれではなく
    /// [`Session::is_trained`]** で、そちらはセットしか見ない。
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty() && self.note.trim().is_empty()
    }
}

/// 1 日分の記録。**1 日 = 1 セッション、1 日 1 種目 1 ログ。**
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Session {
    /// 不変条件: `exercise_id` は重複しない（[`crate::core::migrate`] が正規化する）
    pub logs: Vec<ExerciseLog>,
    #[serde(default)]
    pub body_weight: Option<f32>,
    #[serde(default)]
    pub note: String,
}

impl Session {
    /// 何も入っていない = 保存する価値がない（過去日を閲覧しただけの空セッション）。
    ///
    /// 体重やメモだけが入っているセッションは「空」ではない（破棄すると
    /// コンディションのみの記録が消える）。
    pub fn is_empty(&self) -> bool {
        self.logs.is_empty() && self.body_weight.is_none() && self.note.trim().is_empty()
    }

    /// カレンダーの「実施日」判定。セット付きのログが 1 つでもあるか。
    ///
    /// ★ **メモだけのログは実施日にしない。** メモはトレーニングをした証拠ではないので、
    /// ドット・月フッタ・グラフ・経過日数・「前回」はどれもこの式（セットだけを見る）を
    /// 通す。[`ExerciseLog::is_empty`] と混同しないこと — あちらは「保存する価値が
    /// あるか」で、メモだけのログは**保存はするが実施日にはしない**。
    pub fn is_trained(&self) -> bool {
        self.logs.iter().any(|l| !l.sets.is_empty())
    }

    /// その種目の当日のログ。
    pub fn log_of(&self, ex: ExerciseId) -> Option<&ExerciseLog> {
        self.logs.iter().find(|l| l.exercise_id == ex)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Db {
    pub schema: u32,
    pub groups: Vec<Group>,
    pub exercises: Vec<Exercise>,
    /// 保存済みのトレーニングメニュー。**`Vec` の順が表示順。**
    ///
    /// ★ `skip_serializing_if` は互換のため。メニューを 1 本も作っていない利用者の
    /// JSON は**今までとバイト単位で同一**になり、`e2e/backup.spec.mjs` の
    /// `toEqual(parsed)` などが「保存形式は変わっていない」ことを主張し続ける。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routines: Vec<Routine>,
    /// "YYYY-MM-DD" → 日付順が自動で保たれる（ゼロ埋め ISO なので辞書順 = 時系列順）
    pub sessions: BTreeMap<String, Session>,
}

impl Default for Db {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            groups: Vec::new(),
            exercises: Vec::new(),
            routines: Vec::new(),
            sessions: BTreeMap::new(),
        }
    }
}

impl Db {
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn exercise(&self, id: ExerciseId) -> Option<&Exercise> {
        self.exercises.iter().find(|e| e.id == id)
    }

    pub fn routine(&self, id: RoutineId) -> Option<&Routine> {
        self.routines.iter().find(|r| r.id == id)
    }

    /// アーカイブ済みも含む、その部位の全種目 ID。
    ///
    /// アーカイブ済みを外すと部位グラフ・ドット色・チップが過去分だけ欠ける。
    pub fn exercise_ids_of_group(&self, g: GroupId) -> Vec<ExerciseId> {
        self.exercises
            .iter()
            .filter(|e| e.group_id == g)
            .map(|e| e.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type G = Id<GroupTag>;
    type E = Id<ExerciseTag>;

    #[test]
    fn id_round_trips_through_its_string_form() {
        for bits in [0, 1, RESERVED_MAX, 0x0F_FFFF_FFFF_FFFF, ID_MASK] {
            let id = G::from_bits(bits);
            let text = id.to_string();
            assert_eq!(text.parse::<G>().expect("自分が書いた文字列"), id, "{text}");
        }
    }

    #[test]
    fn id_is_always_twelve_chars() {
        for bits in [0, 1, 31, 32, ID_MASK] {
            assert_eq!(G::from_bits(bits).to_string().len(), ID_LEN);
        }
    }

    #[test]
    fn from_bits_drops_anything_above_sixty_bits() {
        // 上位ビットを残すと Display が落とすので文字列との往復が壊れる
        assert_eq!(G::from_bits(u64::MAX).bits(), ID_MASK);
        assert_eq!(G::from_bits(1 << 60).bits(), 0);
    }

    #[test]
    fn parsing_rejects_wrong_length_or_alphabet() {
        for bad in [
            "",
            "0",
            "00000000001",   // 11 文字
            "0000000000123", // 13 文字
            "00000000001i",  // i は Crockford の除外文字
            "00000000001l",
            "00000000001o",
            "00000000001u",
            "00000000001-",
            "00000000001A", // 大文字は生成しないので受けない
            "あいうえおかきくけこさし",
        ] {
            assert_eq!(
                bad.parse::<G>(),
                Err(IdParseError),
                "{bad:?} を通してしまった"
            );
        }
    }

    #[test]
    fn serde_uses_the_string_form_and_rejects_numbers() {
        // 42 = 1×32 + 10 なので下 2 桁が "1a" になる
        let id = G::from_bits(42);
        let json = serde_json::to_string(&id).expect("直列化できる");
        assert_eq!(json, "\"00000000001a\"");
        assert_eq!(serde_json::from_str::<G>(&json).expect("読み戻せる"), id);

        // ★ 数値を通すと version dispatch を素通りした schema 2 のデータが混入する
        assert!(serde_json::from_str::<G>("42").is_err());
        assert!(serde_json::from_str::<G>("\"42\"").is_err());
        assert!(serde_json::from_str::<G>("null").is_err());
    }

    #[test]
    fn idgen_is_deterministic_for_a_seed() {
        let mut a = IdGen::from_seed(1);
        let mut b = IdGen::from_seed(1);
        let mut c = IdGen::from_seed(2);

        let from_a: Vec<G> = (0..8).map(|_| a.alloc()).collect();
        let from_b: Vec<G> = (0..8).map(|_| b.alloc()).collect();
        let from_c: Vec<G> = (0..8).map(|_| c.alloc()).collect();

        assert_eq!(
            from_a, from_b,
            "同じシードなら同じ列でなければテストが書けない"
        );
        assert_ne!(from_a, from_c, "違うシードで同じ列だと 2 台が衝突する");
    }

    #[test]
    fn idgen_never_returns_a_reserved_id() {
        let mut ids = IdGen::from_seed(0xDEAD_BEEF);
        for _ in 0..10_000 {
            let id: E = ids.alloc();
            assert!(!id.is_reserved(), "予約領域はプリセットのもの: {id}");
        }
    }

    #[test]
    fn idgen_does_not_repeat_itself_in_a_realistic_run() {
        // 生涯で作る ID は数百個。1 万個で重複が出るなら設計が壊れている
        let mut ids = IdGen::from_seed(7);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let id: G = ids.alloc();
            assert!(seen.insert(id), "重複した ID: {id}");
        }
    }

    // ── メモ（adr/data-model/notes-on-logs-and-sets.md）────────────────────────

    fn log_of(sets: Vec<SetEntry>, note: &str) -> ExerciseLog {
        ExerciseLog {
            exercise_id: E::from_bits(1),
            sets,
            at: None,
            note: note.to_string(),
        }
    }

    #[test]
    fn set_entry_omits_an_empty_note_from_its_json() {
        // ★ バイト一致で見る。ここが崩れるとメモを使っていない利用者の保存データが
        //   変わり、e2e の `toEqual([{weight, reps}])` 3 箇所が落ちる
        let set = SetEntry {
            weight: 60.0,
            reps: 10,
            note: String::new(),
        };
        assert_eq!(
            serde_json::to_string(&set).expect("直列化できる"),
            r#"{"weight":60.0,"reps":10}"#
        );
    }

    #[test]
    fn set_entry_reads_json_written_before_notes_existed() {
        let set: SetEntry =
            serde_json::from_str(r#"{"weight":60.0,"reps":10}"#).expect("メモ以前の形も読める");
        assert_eq!(set.weight, 60.0);
        assert_eq!(set.reps, 10);
        assert_eq!(set.note, "");
    }

    #[test]
    fn exercise_log_omits_an_empty_note_from_its_json() {
        let json = serde_json::to_string(&log_of(Vec::new(), "")).expect("直列化できる");
        assert!(!json.contains("note"), "空メモが出ている: {json}");
    }

    #[test]
    fn exercise_log_reads_json_written_before_notes_existed() {
        let log: ExerciseLog = serde_json::from_str(r#"{"exercise_id":"000000000001","sets":[]}"#)
            .expect("メモも at も無い形が読める");
        assert_eq!(log.note, "");
        assert_eq!(log.at, None);
    }

    #[test]
    fn exercise_log_is_empty_only_without_sets_and_without_a_note() {
        let set = || {
            vec![SetEntry {
                weight: 60.0,
                reps: 10,
                note: String::new(),
            }]
        };
        assert!(log_of(Vec::new(), "").is_empty());
        assert!(!log_of(Vec::new(), "肩が痛い").is_empty());
        assert!(!log_of(set(), "").is_empty());
        assert!(!log_of(set(), "肩が痛い").is_empty());
    }

    #[test]
    fn exercise_log_with_a_whitespace_only_note_is_empty() {
        // 空白だけのメモを「ある」とすると、保存の価値が無いログが残り続ける
        for blank in [" ", "\n", "\t", "　", "  \n "] {
            assert!(log_of(Vec::new(), blank).is_empty(), "{blank:?}");
        }
    }

    // ── トレーニングメニュー（adr/data-model/routines-as-named-exercise-lists.md）──

    #[test]
    fn db_omits_routines_from_its_json_when_there_are_none() {
        // ★ バイト一致で見る。ここが崩れるとメニューを使っていない利用者の保存データが
        //   変わり、e2e の「保存形式は変わっていない」系の assertion が落ちる
        let json = serde_json::to_string(&Db::default()).expect("直列化できる");
        assert_eq!(
            json, r#"{"schema":3,"groups":[],"exercises":[],"sessions":{}}"#,
            "メニューが 0 本のときは routines を書いてはいけない"
        );
    }

    #[test]
    fn db_reads_json_written_before_routines_existed() {
        let db: Db =
            serde_json::from_str(r#"{"schema":3,"groups":[],"exercises":[],"sessions":{}}"#)
                .expect("メニュー以前の形も読める");
        assert!(db.routines.is_empty());
    }

    #[test]
    fn db_round_trips_its_routines() {
        let mut db = Db::default();
        db.routines.push(Routine {
            id: Id::from_bits(0x1_0001),
            name: "胸の日".to_string(),
            exercises: vec![E::from_bits(0x1_0010), E::from_bits(0x1_0011)],
        });
        let json = serde_json::to_string(&db).expect("直列化できる");
        assert_eq!(
            serde_json::from_str::<Db>(&json).expect("読み戻せる"),
            db,
            "{json}"
        );
    }

    #[test]
    fn routine_is_empty_only_without_a_name_and_without_exercises() {
        let routine = |name: &str, exercises: Vec<E>| Routine {
            id: Id::from_bits(1),
            name: name.to_string(),
            exercises,
        };
        let one = || vec![E::from_bits(0x1_0010)];
        assert!(routine("", Vec::new()).is_empty());
        // 名前だけ打って種目を選ぶ前に閉じた状態を消してはいけない
        assert!(!routine("胸の日", Vec::new()).is_empty());
        assert!(!routine("", one()).is_empty());
        assert!(!routine("胸の日", one()).is_empty());
    }

    #[test]
    fn a_routine_with_a_whitespace_only_name_and_no_exercises_is_empty() {
        for blank in [" ", "\n", "\t", "　", "  \n "] {
            let r = Routine {
                id: Id::from_bits(1),
                name: blank.to_string(),
                exercises: Vec::new(),
            };
            assert!(r.is_empty(), "{blank:?}");
        }
    }

    #[test]
    fn a_log_that_only_has_a_note_is_worth_saving_but_is_not_a_trained_day() {
        // ★ 「保存する価値がある」と「トレした」は別。前者は is_empty、後者は is_trained
        let session = Session {
            logs: vec![log_of(Vec::new(), "肩が痛いのでやめた")],
            body_weight: None,
            note: String::new(),
        };
        assert!(!session.is_empty(), "メモを黙って捨ててはいけない");
        assert!(
            !session.is_trained(),
            "メモだけの日にカレンダーのドットを点けてはいけない"
        );
    }
}
