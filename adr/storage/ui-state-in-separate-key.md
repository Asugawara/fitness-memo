# UI の状態を `Db` に入れず別キーに置く

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: storage
- **関連**: [localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md), [JSON エクスポート/インポートを v1 に入れない](defer-export-import.md), [保存キーを schema 世代ごとに切り、旧キーを読み取り専用で残す](storage-key-per-schema-generation.md), [ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする](../ux/install-guide-banner-and-sheet.md)

## 背景

[ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする](../ux/install-guide-banner-and-sheet.md) のインストール案内バナーに「今後表示しない」の ✕ を付けた。押したことを覚えておく必要がある。

このアプリの永続化は `localStorage` の単一キー `fitness-memo/v2` に `Db` 全体を JSON で置く形（[localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md)）で、そこにフラグを足すのが素直に見える。しかし 2 つの制約に当たる。

- [JSON エクスポート/インポートを v1 に入れない](defer-export-import.md) は「**`Db` の JSON がそのままエクスポート形式になる**という前提を維持するため、`Db` に UI 都合のフィールドを足さない方針が必要になる」と明記している。実際 `Db` は `schema` / `next_id` / `groups` / `exercises` / `sessions` だけで表示状態を一切持っていない
- [保存キーを schema 世代ごとに切り、旧キーを読み取り専用で残す](storage-key-per-schema-generation.md) により `Db` のフィールド増減は schema 世代の話になる。バナーを消したかどうかのために保存キーを切るのは釣り合わない

一方で [localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md) は「単一キー」を決めている。額面どおり読むと別キーは方針違反である。

## 決定

**UI の状態は `fitness-memo/ui/v1` という別キーに置く。`Db` には入れない。**

```rust
const UI_KEY: &str = "fitness-memo/ui/v1";

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct UiState {
    #[serde(default)]
    install_hint_dismissed: bool,
}

pub fn install_hint_dismissed() -> bool;
pub fn dismiss_install_hint();
```

このキーに 3 つの性質を課す。

1. **`Db` を一切参照しない。** ID も日付キーも入れない
2. **失われても害がない内容だけを置く。** 読めなければ既定値に戻るだけで済むものに限る
3. **移行も退避も持たせない。** `LEGACY_KEYS` にも `.bak-` にも関与しない。読めなければ `Default` で始める

## 理由

- **[localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md) の単一キー方針は、このキーには及ばない。** あの ADR が単一キーにした理由は「複数キーに分けると `ExerciseLog.exercise_id` が存在しない種目を指す状態を作れてしまい、参照整合性を実行時に検証するコードが必要になる」ことだった。**守っていたのは `Db` の内部整合性である。** `Db` を参照しないフラグを別キーに置いても、その整合性はどこも壊れない。方針の文言ではなく方針の理由に照らして判断した。
- **失敗しても害がない。** `set_item` が失敗しても、キーが消えても、JSON が壊れても、起きるのは「案内がもう一度出る」だけである。だから [パース失敗時は上書きせず退避する](quarantine-on-parse-failure.md) の退避も [保存キーを schema 世代ごとに切り、旧キーを読み取り専用で残す](storage-key-per-schema-generation.md) の世代切りも要らない。**回復手段の要否は、失ったときに何が起きるかで決まる。**
- **`Db` に入れるとエクスポートに混入する。** [JSON エクスポート/インポートを v1 に入れない](defer-export-import.md) が本命として挙げているエクスポート／インポートは `Db` の JSON をそのまま出す設計なので、UI フラグを足すとバックアップファイルに「バナーを消したか」が入る。さらに他端末へインポートしたときに、その端末では消していない案内が消える。**UI の状態は端末に属していてデータに属していない。**
- **`Db` に入れると schema 世代を切る話になる。** フィールドを足すだけなら `#[serde(default)]` で読めるが、[保存キーを schema 世代ごとに切り、旧キーを読み取り専用で残す](storage-key-per-schema-generation.md) が「フィールドを消す変更は前方互換を壊す」と整理した領域に UI の都合を持ち込むことになる。将来 UI フラグを 1 つ消すたびに保存キーの世代を検討するのは割に合わない。
- **debounce しない。** `save_debounced` は 1 文字入力ごとの直列化を避けるためのもので、クリック 1 回で終わるフラグには要らない。`visibilitychange` の flush（[`visibilitychange` の hidden で debounce を flush する](flush-on-visibilitychange.md)）とも無関係である。
- **キー名に世代 `v1` を入れた。** 使う予定は無いが、`fitness-memo/` 名前空間の他のキーと形を揃えておくほうが、後から見て「これは何世代目か」を考えずに済む。

## 結果（トレードオフ）

- **`localStorage` のキーが 2 本になった。** 「このアプリのデータは 1 キーに全部入っている」と読める [localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md) の記述が、そのままでは正しくなくなった。この ADR からの相互リンクで補っている。
- **境界の判断を人間が守る必要がある。** 「`Db` を参照しない」「失われても害がない」は型で強制できない。`UiState` に `last_selected_exercise: ExerciseId` のようなフィールドを足した瞬間に前提が崩れる（`Db` から種目が消えたときに宙に浮く）。`src/storage.rs` の該当ブロックにコメントで条件を書いてあるが、レビューで見るしかない。
- **エクスポートの対象外になる。** バックアップを取って別端末へ復元しても、UI の状態は移らない。今回の内容（案内を消したか）では望ましい挙動だが、将来「移ってほしい UI 設定」が出てきたらこの置き場では扱えない。
- **`store()` が使えない環境では機能しない。** Safari のプライベートブラウズでは `local_storage()` が例外を投げるので、✕ を押してもその場で消えるだけで次回また出る。ただしその環境では `Db` も保存されないので、案内が出続けること自体は害にならない。
- **E2E で「`Db` に混ざっていないこと」を固定した。** `fitness-memo/v2` の JSON に `install_hint` が含まれないことをテストしている。これが無いと、後から誰かが `Db` へ移しても気付けない。

## 検討した代替案

**`Db` に `install_hint_dismissed: bool` を足す**: 保存も読み込みも既存経路に乗るので実装が最小。しかし [JSON エクスポート/インポートを v1 に入れない](defer-export-import.md) が明示した「`Db` に UI 都合のフィールドを足さない」に正面から反し、エクスポートファイルに UI 状態が混入する。別端末へのインポートで案内が消える副作用も付く。却下。

**`sessionStorage` に置く**: `Db` を汚さず、キー設計も要らない。しかしタブを閉じると消えるので、要件の「二度と表示されない」を満たさない。却下。

**キーを分けず、値を持たないキーの有無で表す**（`localStorage.setItem("fitness-memo/install-hint-dismissed", "1")`）: JSON も serde も要らず最小。しかし UI のフラグが増えるたびにキーが増え、`fitness-memo/` 名前空間が散らかる。1 つの JSON にまとめておけば追加は 1 フィールドで済む。却下。

**そもそも消せなくする（✕ を付けない）**: この ADR 自体が不要になる。データ損失の警告としては最も安全でもある。しかし [ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする](../ux/install-guide-banner-and-sheet.md) のとおり利用者が明示的に ✕ を求めた。却下。

**Cookie / IndexedDB を使う**: どちらも `Db` と保存先を分けられる。しかし `localStorage` はすでに使っていて同期 API であり、この用途に非同期の IndexedDB を持ち込む理由がない。Cookie は送信先が無いアプリでは単に不適切。却下。
