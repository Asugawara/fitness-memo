# fitness-memo アイコン生成指示書（image-2 向け）

> このファイルはリポジトリ外（scratchpad）にあります。git には乗りません。

## コンセプト

筋トレの器具（ダンベル・バーベル）は描かない。
**ホーム画面で目に入るたびに「やるか」と思える、情熱と上昇のエネルギー**を抽象で表す。
記録アプリの実務感ではなく、火をつける側の絵。

---

## 0. 技術要件（これを外すと使えない画像になる）

| 項目 | 要件 | 理由 |
|---|---|---|
| 出力サイズ | **1024×1024 ちょうど**（正方形 1:1） | 192/512 に縮小して使う |
| 背景 | **全面塗り・四辺の端まで到達・透過なし** | iOS は透過を黒に合成する |
| 角丸 | **付けない**（角は直角） | iOS が独自の角丸マスクをかける。二重角丸になる |
| 枠・縁取り・余白 | **付けない** | マスクで切られて中途半端に残る |
| 主要素の占有 | 画面幅の **55〜60%**・中央配置 | maskable のセーフゾーン（中央 80% 円内）に収める |
| 文字 | **一切なし** | 生成モデルが必ず崩す |
| 可読性 | **60×60px でも形が読める**質量 | ホーム画面の実表示サイズ |

主要素を中央 60% に収めれば、**1 枚の画像を `any` と `maskable` の両方に使い回せます**
（変換工程が `resize` だけで済む）。

---

## 1. モチーフ案

4 案とも「背景 = 全面塗り、前景 = 単一の抽象シルエット」の構成です。
まず **案 A** を第一候補として出し、気に入らなければ B〜D を試してください。

### 案 A: 幾何学的な炎（第一候補）

「情熱」の直球。単一シルエットなので 60px でも潰れない。

```
A minimalist mobile app icon representing passion and drive.

COMPOSITION: Perfect square, 1:1 aspect ratio, 1024x1024. The artwork
bleeds fully to all four edges — no border, no frame, no rounded corners,
no transparency, no icon-inside-a-frame. Do not draw a smaller icon shape
floating on a canvas; the background IS the icon.

SUBJECT: A single stylized flame, built from clean geometric curves and
one bold teardrop silhouette with a smaller inner tongue. Rising upward,
perfectly centered, symmetrical and confident. Solid fill in warm white
(#FFF6EE), no outline, no gradient inside the flame itself.

SCALE: The flame spans about 45% of the icon width and 58% of its height,
sitting optically centered with wide calm margins on all four sides. It
must stay unmistakable at 60x60 pixels.

BACKGROUND: A smooth vertical gradient from deep crimson #7A1020 at the
bottom to vivid orange #F2661F at the top. Even and clean — no noise, no
texture, no vignette, no rays.

STYLE: Modern flat design, geometric precision, high contrast, crisp hard
edges. No bevel, no emboss, no drop shadow, no glow, no gloss, no 3D
rendering, no realistic fire, no smoke, no sparks, no embers. Absolutely
no text, no letters, no numbers.
```

### 案 B: 急上昇するライン

「上がっていく」を直接描く。記録アプリの文脈とも繋がる。

```
SUBJECT: A single bold ascending polyline of three connected straight
segments, climbing steeply from the lower-left toward the upper-right,
ending in a solid triangular arrowhead. Thick uniform stroke with
mitered corners, solid warm white (#FFF6EE) fill, no outline. Pure
geometry, no curves.

SCALE: The polyline spans about 58% of the icon width, optically
centered with wide calm margins on all four sides.
```
背景・COMPOSITION・STYLE は案 A の同名ブロックをそのまま使う。

### 案 C: 昇る太陽

「これから上がる」の比喩。放射を入れるので 60px 検証は必須。

```
SUBJECT: A solid half-disc sun rising from a straight horizon line near
the lower third, with five short thick rays radiating above it. All
shapes solid warm white (#FFF6EE), flat, no outline, strictly symmetrical
about the vertical axis. Bold and simple.

SCALE: The whole sun-and-rays group spans about 58% of the icon width,
optically centered.
```

### 案 D: 稲妻

瞬発力。最もシャープで小サイズに強い。

```
SUBJECT: A single bold lightning bolt, one continuous angular zigzag
silhouette with sharp clean vertices, tilted slightly clockwise. Solid
warm white (#FFF6EE) fill, generous thickness, no outline, no glow.

SCALE: The bolt spans about 40% of the icon width and 60% of its height,
optically centered.
```

---

## 2. ネガティブプロンプト（全案共通）

```
text, letters, numbers, typography, watermark, signature, logo type,
rounded corners, border, frame, padding, canvas edges, drop shadow, glow,
bevel, emboss, gloss, glossy highlight, metallic reflection, chrome,
3D render, photorealistic, photograph, realistic fire, smoke, sparks,
embers, person, hand, muscle, body, dumbbell, barbell, gym equipment,
cluttered, busy composition, multiple objects, thin lines, outline stroke,
sketch, hand-drawn, texture, noise, grain, gradient banding,
transparent background, checkerboard
```

---

## 3. 配色案（`BACKGROUND:` ブロックだけ差し替える）

**1. クリムゾン → オレンジ（第一候補・炎に一番合う）**
```
A smooth vertical gradient from deep crimson #7A1020 at the bottom to
vivid orange #F2661F at the top.
```

**2. ディープレッド → アンバー（落ち着いた熱さ）**
```
A smooth diagonal gradient from deep red #8C1D18 at the bottom-left to
warm amber #E0912A at the top-right.
```

**3. 紫 → ピンク → オレンジ（サンセット・一番派手）**
```
A smooth vertical gradient from deep violet #3B1E5E at the bottom through
magenta #C2298A in the middle to warm orange #F2661F at the top.
```

**4. 濃紺 → 燃えるオレンジ（暗い地に炎が映える・締まる）**
```
A near-black indigo #14121C at the bottom fading into a warm orange glow
#F2661F concentrated in the upper half.
```

> アプリ本体のアクセントは青（`#2f6fd0`）ですが、アイコンは寒色に揃えません。
> ホーム画面は青系アプリが密集する場所なので、暖色のほうが見つけやすくなります。
> アプリ内の配色は変更しません（`theme-color` は据え置き）。

---

## 4. 受け入れチェック（全部通ったら採用）

1. 四隅まで背景色が来ているか（白い余白・角丸・透過が混ざっていないか）
2. **60×60px に縮小しても**モチーフが何か分かるか ← 一番落ちやすい
3. 中央 80% の円からモチーフがはみ出していないか
4. 文字・記号・意図しない小さな装飾が紛れていないか
5. 出力が 1024×1024 ちょうどの正方形か
6. ダークな壁紙・明るい壁紙どちらの上でも沈まないか

チェック 2 はコマンドで確認できます:
```sh
magick <生成画像> -resize 60x60 -resize 600x600 /tmp/icon-60px-check.png
```

---

## 5. 生成後の組み込み（画像パスを渡してもらえればこちらで実行します）

```sh
# 1. 正方形 1024 のマスターを作る
magick <生成画像> -gravity center -crop 1:1 +repage \
       -resize 1024x1024 -alpha remove -alpha off -strip \
       public/icons/icon-master.png

# 2. 各サイズを書き出す（主要素が中央 60% なので maskable も同じ絵でよい）
magick public/icons/icon-master.png -resize 192x192 -strip public/icons/icon-192.png
magick public/icons/icon-master.png -resize 512x512 -strip public/icons/icon-512.png
cp public/icons/icon-512.png public/icons/icon-maskable-512.png
```

あわせて実施:
- `scripts/gen-icons.sh` を「SVG → rsvg-convert」から「マスター PNG → magick」に書き換え
- 旧 `public/icons/icon.svg` を削除
- `public/manifest.webmanifest` の `background_color`（起動スプラッシュの地色）を
  アイコン下端の色に合わせる
- `index.html` の `theme-color` は**触らない**（アプリ本体の地色に対応しているため）
- `scripts/stamp-sw.sh` が staging 全ファイルから BUILD_ID を導出するので、
  SW キャッシュは自動で更新される（手当て不要）
