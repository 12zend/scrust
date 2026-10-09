# PRODUCT.md — scrust: Scratch 3 → Rust AOT コンパイラ

> この文書は **実装エージェント向けの仕様兼プロンプト** である。上から順に読み、§16 のフェーズ順に実装すること。
> 仕様の最終的な正（ground truth）は **scratch-vm（バニラ Scratch 3）の挙動** である。TurboWarp は「最適化の手本」であって挙動の正ではない。両者が食い違う箇所は §2.3 に列挙してある。
> TurboWarp のフォークではなく、一から書く。TurboWarp / scratch-vm のコードは**読んで挙動を確認する目的でのみ**参照し、コピーしない（ライセンス: scratch-vm は AGPL-3.0 系/BSD 系が混在するため、移植ではなく仕様の再実装とする）。

---

## 0. 実装エージェントへの指示（最重要）

1. **型と値のセマンティクスが本プロジェクトの核心**。描画や音より先に、§4〜§9 を完璧にすること。見た目が動いても `"1.0"` と `1` の区別を間違えればプロジェクトは壊れる。
2. 仕様に迷ったら推測しない。scratch-vm の該当ソース（§18）を読み、**差分テストハーネス（§15）で実測**して決める。決めた結果は `docs/decisions/` に1件1ファイルで記録する。
3. 着手時に scratch-vm と TurboWarp/scratch-vm の参照コミットハッシュを `REFERENCE.lock` に固定する。以後、挙動の議論はそのコミット基準で行う。
4. 「要検証」と書かれた項目は、実装前に必ず差分テストで確認すること。
5. 各フェーズの Done 定義（§16）を満たすまで次へ進まない。

---

## 1. プロダクト概要

### 1.1 何を作るか
`scrust` は `.sb3`（Scratch 3 プロジェクト）を入力として受け取り、**Rust のソースコード（Cargo クレート）を生成**し、それを `cargo` でビルドして**ネイティブ実行ファイル**または **wasm32 + Web ページ**にするコンパイラである。

```
project.sb3 ──parse──▶ AST ──lower──▶ IR ──type analysis──▶ Typed IR ──codegen──▶ Rust crate ──cargo──▶ exe / wasm
                                                                                        │
                                                                         depends on scrust-runtime
```

### 1.2 ゴール
- **G1 互換性**: バニラ Scratch 3 と観測可能な挙動が一致すること（変数値、`say` の文字列、リスト内容、スレッド実行順、フレーム内の進み方）。
- **G2 速度**: TurboWarp（JS JIT）と同等以上。数値しか入らない変数は `f64` に、文字列しか入らない変数は文字列型に**アンボックス化**し、キャストを静的に消去する。
- **G3 決定性**: テストモードでは乱数シード・仮想時計・固定フレームで完全に再現可能。
- **G4 移植性**: Windows / macOS / Linux ネイティブ、および wasm32 (ブラウザ)。

### 1.3 非ゴール（初期）
- エディタ（ブロック編集 UI）。scrust は**プレイヤー＋コンパイラ**のみ。
- Scratch 2 (`.sb2`) 読み込み（Phase 6 以降の任意項目）。
- ハードウェア拡張（micro:bit, LEGO 等）、ビデオモーション、音声認識、翻訳、読み上げ。
- TurboWarp 独自拡張（カスタム拡張 JS の実行は原理的に不可）。
- クラウド変数のサーバ通信（ローカル変数として扱う。Phase 6 で任意）。

### 1.4 スコープ内の拡張
- **ペン（pen）** は必須（多くのプロジェクトが依存）。
- 音楽（music）は Phase 6 の任意。

---

## 2. 準拠基準

### 2.1 正（ground truth）
`scratchfoundation/scratch-vm` の `develop` ブランチ（コミット固定）＋ `scratch-render` ＋ `scratch-audio`。

### 2.2 JavaScript セマンティクスへの依存
Scratch の値は JS の値そのもの（`number`/`string`/`boolean`）であり、キャストは JS の `Number()`、`String()`、`toLowerCase()`、`<` 比較、`Math.*` に依存している。したがって scrust は**以下の ECMAScript アルゴリズムを Rust で忠実に再実装**しなければならない（§5 で詳述）:

| JS の機能 | 使われる箇所 | Rust 標準との差 |
|---|---|---|
| `Number(string)`（StringToNumber） | 全ての数値キャスト、比較 | `f64::from_str` は `"inf"`, `"NaN"`, `"infinity"` を受理し、`" 12 "`, `"0x10"`, `""` を拒否する → **使用禁止**（下請けのみ可） |
| `String(number)`（Number::toString） | 文字列化全般、`say`、`join` | Rust の `{}` は `1e21`→`"1000000000000000000000"`、`-0`→`"-0"`、`NaN`→`"NaN"`、`inf` → **使用禁止** |
| 文字列 = UTF-16 コード単位列 | `length`、`letter of`、比較、単一文字判定 | Rust の `String` は UTF-8 で孤立サロゲートを持てない |
| `toLowerCase()` | 比較、`contains` | Unicode 既定ケースマッピング（特殊ケース含む） |
| 文字列 `<` | 比較 | UTF-16 コード単位の辞書順（UTF-8 のバイト順とは異なる） |
| `Math.round` | `round`、`repeat`、コスチューム番号 | **0.5 は +∞ 方向**。Rust の `f64::round` は 0 から遠い方向 → **使用禁止** |
| `%` | `mod` | Rust の `f64 %` と同一（fmod） |
| `Math.sin/cos/...` | 三角関数 | プラットフォーム libm 依存 → `libm` クレート（fdlibm 系）で固定 |
| `Number.prototype.toFixed(10)` + `parseFloat` | `sin`/`cos`/`tan` | 正確な10進丸め（タイは大きい方）を実装 |
| `JSON.parse` | sb3 読み込み | オブジェクトキー順序（整数風キーが先頭）を再現 |

### 2.3 バニラと TurboWarp の既知の差（scrust はバニラに従う）
| 項目 | バニラ | TurboWarp | scrust 既定 |
|---|---|---|---|
| リスト内容の文字列化（`data_listcontents`） | 全要素が **型 string かつ長さ1** なら区切りなし、それ以外は空白区切り | `String(item).length === 1` で判定（数値 `1` も1文字扱い） | **バニラ**。ただし §8.6 の「正準数値文字列の消去」をこの経路では無効化 |
| `tan` の丸め | `parseFloat(Math.tan(x).toFixed(10))` | `Math.round(x*1e10)/1e10` | バニラ |
| 比較の「実は0でない」判定 | `Number(v)===0 && v.trim()===''` なら NaN 扱い（`else if` で片側のみ） | 文字列に `'0'` も TAB も含まなければ NaN 扱い（`"\t"` を 0 とみなす） | バニラ |
| warp タイマー（500ms で warp 中も yield） | 常に有効 | 既定無効（エディタ内のみ有効） | オプション `--warp-timer`（既定: **無効**、`--compat=strict` で有効） |
| 変数の初期型 | ブロック定数は文字列 `"10"` のまま格納 | 一部定数を数値に変換 | バニラ（ただし §8.6 で観測不能な場合のみ数値化） |

---

## 3. アーキテクチャ

### 3.1 Cargo ワークスペース
```
scrust/
├── crates/
│   ├── scrust-value/       # Value, JsStr, キャスト, 数値⇔文字列, 比較, Math（依存最小・no_std+alloc 可）
│   ├── scrust-sb3/         # sb3 (zip + project.json) のパース → 生 AST
│   ├── scrust-ir/          # AST → IR への lowering、名前解決、定数畳み込み
│   ├── scrust-analysis/    # 型推論（§8）、yield 解析、手続きバリアント決定
│   ├── scrust-codegen/     # Typed IR → Rust ソース生成
│   ├── scrust-runtime/     # スケジューラ、World、スプライト/クローン、イベント、ブロック実装
│   ├── scrust-render/      # wgpu 描画、SVG/ビットマップ、ペン、吹き出し、当たり判定
│   ├── scrust-audio/       # 音声デコード・再生・エフェクト
│   ├── scrust-player/      # winit ウィンドウ / web ホスト、入力、モニター表示、ask 入力
│   └── scrust-cli/         # `scrust` コマンド
├── conformance/            # 差分テスト（Node + scratch-vm ハーネス、fuzzer、ゴールデン）
└── docs/decisions/
```
`scrust-value` は他に依存せず単体で徹底テストする。生成コードと runtime は必ずこれを経由して値を扱う。

### 3.2 CLI
```
scrust build <in.sb3> [-o out/] [--target native|wasm] [--release] [--compat=default|strict]
             [--warp-timer] [--fps 30|60|N] [--turbo] [--deterministic --seed N]
scrust run   <in.sb3> [...]          # build して即実行
scrust emit  <in.sb3> -o crate/      # Rust クレートを出力するだけ
scrust check <in.sb3>                # 未対応ブロック・拡張の一覧、型推論レポート
scrust trace <in.sb3> --frames N     # ヘッドレス実行し、差分テスト用トレース(JSON Lines)を出力
```

### 3.3 生成クレート
- `Cargo.toml`（`scrust-runtime` に依存）、`src/main.rs`、スプライトごとに `src/sprite_<n>.rs`、アセットは `include_bytes!` で埋め込み（`--assets external` で外部ファイル化）。
- 巨大プロジェクト対策: 1関数あたりの生成行数に上限（例: 2,000 行）を設け、超えたらサブ関数に分割。`[profile.release] codegen-units`、`opt-level` を調整可能に。

---

## 4. 値モデル（核心）

### 4.1 Scratch の値
Scratch 3 の変数・リスト要素・レポーター戻り値は次の3種のいずれか:

- **number**（IEEE 754 binary64。`NaN`, `±Infinity`, `-0` を含む）
- **string**（UTF-16 コード単位列。孤立サロゲートを含み得る）
- **boolean**

`undefined`/`null` は通常のプロジェクトでは値として現れない（空のブール入力スロットは `false` として扱う。§7.5）。

### 4.2 Rust 表現
```rust
/// 動的型の値。16 バイトに収めること（JsStr を thin pointer にする）。
#[derive(Clone)]
pub enum Value {
    Num(f64),
    Str(JsStr),
    Bool(bool),
}
```

### 4.3 JsStr（JS 文字列互換の不変文字列）
要件:
- **コード単位は UTF-16**。`len()` は UTF-16 単位数で O(1)、`at(i)` は O(1)。
- 孤立サロゲートを保持できる（`letter 1 of "😀"` は上位サロゲート単体を返すのがバニラの挙動）。
- 不変・参照カウント（`Rc`、シングルスレッド）。clone は O(1)。
- **Latin-1 / UTF-16 のハイブリッド**（V8 の one-byte/two-byte 文字列と同じ発想）: 全コード単位が `<= 0xFF` なら `u8` 配列、そうでなければ `u16` 配列。
- 小文字列の最適化: 1文字文字列（`letter of` の結果に多い）は 0x00–0xFF をスタティックテーブルで共有。
- **連結のロープ化**: `join` をループで繰り返すプロジェクトは多く、素朴実装は O(n²)。連結結果は `Cons(left, right, len)` ノードとし、インデックス・比較・ハッシュ・出力時に一度だけ平坦化（V8 の ConsString と同等）。平坦化後はノードを置換してキャッシュ。
- 比較は UTF-16 コード単位の辞書順。Latin-1 同士は `memcmp`。
- 表示（描画・stdout）時のみ UTF-8 に変換し、孤立サロゲートは U+FFFD に置換。

```rust
pub struct JsStr(/* thin Rc pointer to header{kind, len, hash_cache} + data */);
impl JsStr {
    pub fn len(&self) -> usize;                 // UTF-16 code units
    pub fn code_unit(&self, i: usize) -> u16;
    pub fn concat(&self, other: &JsStr) -> JsStr; // rope
    pub fn to_lower(&self) -> JsStr;            // §5.6
    pub fn cmp_utf16(&self, other: &JsStr) -> Ordering;
    pub fn from_static(s: &'static str) -> JsStr; // 定数用、const 初期化可能に
}
```

### 4.4 数値
- 常に `f64`。整数専用表現（`i32`/`i64`）は**値の型としては導入しない**（2^53 超え、`-0`、オーバーフロー挙動が JS と変わるため）。
- 例外: コード生成器がループカウンタ等で**値域を証明できた場合のみ**局所的に `i64`/`usize` を使ってよい（§9.4）。

---

## 5. キャストと基本アルゴリズムの完全仕様

すべて `scrust-value` に実装し、各関数に §15.3 の表のテストを付けること。

### 5.1 `to_number(v) -> f64`（= `Cast.toNumber`）
```
Num(n)  => if n.is_nan() { 0.0 } else { n }      // -0 は -0 のまま保持する
Bool(b) => if b { 1.0 } else { 0.0 }
Str(s)  => let n = js_string_to_number(s); if n.is_nan() { 0.0 } else { n }
```
**重要**: `NaN` は「数値として使われた瞬間に 0 になる」。`(0/0)` を `say` すると `"NaN"` だが、`(0/0) * 1` は `0`。変数に格納された `NaN` は `NaN` のまま（`change by` で 0 扱い）。

### 5.2 `js_string_to_number(s) -> f64`（ECMAScript StringToNumber）
1. 両端の **JS 空白**を除去: U+0009, U+000A, U+000B, U+000C, U+000D, U+0020, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF（Unicode Zs 全体 + LineTerminator + BOM。Unicode バージョン差に注意）。
2. 空なら `0`。
3. `^[+-]?Infinity$` → `±∞`（大文字小文字区別。`"infinity"` は NaN）。
4. `^0[xX][0-9a-fA-F]+$`, `^0[oO][0-7]+$`, `^0[bB][01]+$` → 整数値を**正しく丸めて** f64 化（2^53 超も正しく丸める）。**符号は付けられない**（`"-0x10"` は NaN）。
5. 10進: `^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$` に完全一致するもののみ。`"1."`, `".5"`, `"1e5"`, `"+3"` は有効。`"."`, `"1_000"`, `"1,000"`, `"١٢"`（非ASCII数字）, `"e5"` は NaN。
6. 一致したら正しく丸めた f64 に変換（文法検証後なら `f64::from_str` を下請けに使ってよい。指数が巨大な場合 `±∞` / `±0` になることを確認）。
7. いずれにも一致しなければ NaN。
8. 結果の `-0`（`"-0"`, `"-0.0"`）は保持。

高速パス: Latin-1 で短い ASCII 数字列は専用の手書きパーサ（正しく丸まる範囲: 15 桁以下かつ指数なし等）で処理。

### 5.3 `to_string(v) -> JsStr`（= `String(v)`）
- `Str(s)` → そのまま。
- `Bool` → `"true"` / `"false"`。
- `Num(n)` → **ECMAScript Number::toString(10)**:
  - `NaN` → `"NaN"`、`±0` → `"0"`（**`-0` も `"0"`**）、`+∞` → `"Infinity"`、`-∞` → `"-Infinity"`。
  - それ以外は最短往復表現の桁 `d` と指数 `n` を求め、`1e-7 <= |x| < 1e21` なら固定小数表記、それ以外は `"1e+21"`, `"1.5e-7"` 形式（指数の `+` 必須）。
  - 実装は `ryu-js` クレート相当のアルゴリズム（ECMAScript 準拠をテストで確認）。整数（`|x| < 2^53` かつ整数）は高速パスで itoa。
- 例: `0.1+0.2` → `"0.30000000000000004"`, `1e21` → `"1e+21"`, `123456789012345680000` → `"123456789012345680000"`, `0.000001` → `"0.000001"`, `1e-7` → `"1e-7"`。

### 5.4 `to_boolean(v) -> bool`（= `Cast.toBoolean`）
- `Bool(b)` → `b`
- `Num(n)` → `!(n == 0.0 || n.is_nan())`（`-0` も false）
- `Str(s)` → `s == "" || s == "0" || s.to_lower() == "false"` なら false、それ以外 true。
  - `"0.0"`, `" 0"`, `"00"`, `"no"` は **true**。`"FALSE"`, `"False"` は false。

### 5.5 `compare(a, b) -> f64`（= `Cast.compare`、`=`/`<`/`>`/`item # of`/`list contains` の基礎）
```
n1 = js_number(a); n2 = js_number(b)            // NaN を 0 にしない生の Number()
if n1 == 0 && is_whitespace(a) { n1 = NaN }
else if n2 == 0 && is_whitespace(b) { n2 = NaN } // ← else if（バニラ準拠。片側のみ）
if n1.is_nan() || n2.is_nan() {
    s1 = to_string(a).to_lower(); s2 = to_string(b).to_lower();
    return s1.cmp_utf16(s2) as -1/0/1
}
if (n1 == +inf && n2 == +inf) || (n1 == -inf && n2 == -inf) { return 0 }
return n1 - n2
```
- `js_number(Bool)` は 1/0、`js_number(Num(n))` は n（NaN のまま）。
- `is_whitespace(v)` = `v` が文字列かつ JS `trim()` 後に空。
- 結果: `equals` は `== 0`、`lt` は `< 0`、`gt` は `> 0`。
- **帰結（テスト必須）**:
  - `NaN = NaN` は **true**（どちらも `"nan"` として文字列比較）。
  - `"" = 0` は false、`" " = 0` は false、`"0" = ""` は false。
  - `"1.0" = 1` は true、`"1e3" = 1000` は true、`"0x10" = 16` は true。
  - `"abc" = "ABC"` は true。`"10" < "9"` は false（数値比較）、`"10" < "9a"` は true（文字列比較）。
  - `true = "true"` は true（文字列比較）、`true = 1` は true（数値比較）。
  - `Infinity = "Infinity"` は true。
- 型付き高速パス（§9.3）: 両辺が NaN でない f64 と証明できれば `a == b`, `a < b`, `a > b` をそのまま使ってよい（`±∞` も IEEE 比較で正しい）。

### 5.6 小文字化
- JS `toLowerCase()` = Unicode 既定ケースマッピング（SpecialCasing 含む: `İ` → `i̇`（2単位）、語末シグマ `Σ`→`ς`）。
- 実装: Latin-1 は 256 要素テーブル。UTF-16 はサロゲートペアを結合してコードポイント単位でマッピング、孤立サロゲートはそのまま通す。語末シグマの文脈規則も実装（Rust の `str::to_lowercase` と同一規則）。
- Unicode のバージョン差は許容差として `docs/decisions` に記録。

### 5.7 `is_int(v)`（= `Cast.isInt`、`pick random` で使用）
- `Num(n)`: `NaN` は true。それ以外は `n === parseInt(String(n), 10)`。
  - 注意: `1e21` は `String` が `"1e+21"`、`parseInt` が 1 になるため **false**。`1.5` は false、`-0` は `parseInt("0")=0` で true。
- `Bool` → true。
- `Str(s)` → `s` に `'.'` を含まなければ true（`"abc"` も true、`"1e5"` も true）。

### 5.8 `to_list_index(v, len, accept_all)`（= `Cast.toListIndex`）
```
if v is not Num:
    if v is Str("all")  -> if accept_all { ALL } else { INVALID }
    if v is Str("last") -> if len > 0 { len } else { INVALID }
    if v is Str("random") | Str("any") -> if len > 0 { 1 + floor(rand()*len) } else { INVALID }
idx = floor(to_number(v))
if idx < 1 || idx > len { INVALID } else { idx }
```
- 判定は**大文字小文字区別**（`"LAST"` は数値化されて 0 → INVALID）。
- `1.9` → 1、`"2"` → 2、`Infinity` → INVALID。

### 5.9 Math 関数
- `libm` クレートを使い、プラットフォーム非依存に固定する（V8 は fdlibm 系の `ieee754::sin` 等を使うため、ULP 単位でも一致させやすい）。`pow` は V8 独自実装の可能性があるため差分テストで確認（要検証）し、差があれば記録。
- `js_round(x)`（`Math.round`）:
  ```rust
  fn js_round(x: f64) -> f64 {
      if !x.is_finite() || x == 0.0 { return x; }
      if x < 0.0 && x >= -0.5 { return -0.0; }
      let f = x.floor();
      if x - f >= 0.5 { f + 1.0 } else { f }
  }
  ```
  `round(2.5)=3`, `round(-2.5)=-2`, `round(-0.4)=-0`, `round(0.49999999999999994)=0`。
- `js_to_fixed(x, 10)` → 正確な10進展開に基づき、等距離なら**絶対値の大きい方**（負数は絶対値で処理して符号を付ける）。`|x| >= 1e21` は `String(x)`。その結果を `js_string_to_number` で戻す（= `parseFloat`。この経路では入力が常に有効な10進なので等価）。

---

## 6. 演算子ブロック仕様

`N(x)` = `to_number`, `S(x)` = `to_string`, `B(x)` = `to_boolean`。

| opcode | 結果 | 規則 |
|---|---|---|
| `operator_add` | number | `N(a) + N(b)` |
| `operator_subtract` | number | `N(a) - N(b)` |
| `operator_multiply` | number | `N(a) * N(b)` |
| `operator_divide` | number | `N(a) / N(b)`（`1/0=∞`, `0/0=NaN`, `1/-0=-∞`） |
| `operator_mod` | number | `r = N(a) % N(b); if r / N(b) < 0 { r += N(b) }`（床除算）。`mod(-1,3)=2`, `mod(5,-3)=-1`, `mod(x,0)=NaN` |
| `operator_round` | number | `js_round(N(x))` |
| `operator_lt/equals/gt` | boolean | §5.5 |
| `operator_and/or` | boolean | `B(a) && B(b)` / `B(a) \|\| B(b)`。**両辺とも評価される**（入力は事前評価。短絡しない。副作用のあるレポーターは無いが手続き呼び出しの評価順に注意） |
| `operator_not` | boolean | `!B(a)` |
| `operator_join` | string | `S(a).concat(S(b))` |
| `operator_letter_of` | string | `i = N(LETTER) - 1; s = S(STRING); if i < 0 \|\| i >= len { "" } else { s[trunc(i)] }`（`charAt` は小数を切り捨て。`letter 1.9 of "abc"`="a", `letter 0.5 of "abc"`=""） |
| `operator_length` | number | UTF-16 単位数（`"😀"` は 2） |
| `operator_contains` | boolean | `S(a).to_lower()` が `S(b).to_lower()` を部分列として含む（空文字列は常に true） |
| `operator_random` | number | 下記 |
| `operator_mathop` | number | 下記 |

**`pick random`**:
```
from = N(FROM); to = N(TO); low = min; high = max
if low == high { return low }
if is_int(FROM_raw) && is_int(TO_raw) { low + floor(rand() * (high + 1 - low)) }
else { rand() * (high - low) + low }
```
`is_int` は**キャスト前の生の値**に対して判定する。したがって `random "1.0" to 10` は小数、`random 1 to 10` は整数。→ **定数の文字列/数値の区別を消してはならない**典型例（§8.6）。

**`mathop`**（`OPERATOR` を `S().to_lower()` して分岐。未知は 0）:
| op | 実装 |
|---|---|
| abs, floor, ceiling, sqrt | `abs`, `floor`, `ceil`, `sqrt`（`sqrt(-1)=NaN`） |
| sin / cos | `parse(js_to_fixed(libm::sin(PI*n/180), 10))` |
| tan | `a = n % 360; a ∈ {90, -270} → +∞; a ∈ {-90, 270} → -∞; else parse(to_fixed(libm::tan(PI*a/180),10))` ※ バニラの `MathUtil.tan` は `angle % 360` で**上書きした a** を tan に渡す（TurboWarp は元の n を渡すので巨大角度で差が出る）。scrust はバニラに従う |
| asin, acos, atan | `libm::asin(n) * 180 / PI` 等（この順の演算） |
| ln | `ln(n)` |
| log | `ln(n) / LN_10`（**`log10` ではない**。`log 1000 = 2.9999999999999996` を再現） |
| e ^ | `exp(n)` |
| 10 ^ | `pow(10, n)` |

---

## 7. データ・制御ブロックの値仕様

### 7.1 変数
- `set [v] to (x)`: **生の値をそのまま格納**（キャストしない）。`set v to "007"` は文字列 `"007"`。
- `change [v] by (x)`: `v = N(v) + N(x)`（結果は常に number）。
- 参照されたが定義の無い変数 ID は `lookupOrCreateVariable` の挙動（スプライトローカル→ステージを ID、次に名前で検索し、無ければローカルに作る）を**コンパイル時に**解決する。

### 7.2 リスト（上限 200,000 要素）
| ブロック | 規則 |
|---|---|
| add | 長さ < 200000 のときのみ push（生の値） |
| delete (i) | `to_list_index(i, len, true)`。ALL なら全削除。INVALID なら何もしない |
| delete all | 全削除 |
| insert (x) at (i) | `to_list_index(i, len+1, false)`。`idx > 200000` なら何もしない。挿入後に上限を超えたら末尾を pop |
| replace (i) with (x) | `to_list_index(i, len, false)` |
| item (i) | 無効なら `""`（空文字列） |
| item # of (x) | 先頭から `compare(item, x) == 0` の最初の 1-based 位置、無ければ 0 |
| length | number |
| contains (x) | 同一性（`indexOf`、型も値も同一）→ 無ければ `compare == 0` で走査 |
| listcontents（リストのレポーター） | **全要素が「型 string かつ長さ 1」なら区切りなし連結、そうでなければ `" "` 区切り**。各要素は `S()` で文字列化 |

### 7.3 制御
| ブロック | 規則 |
|---|---|
| repeat (n) | 開始時に一度だけ `times = js_round(N(n))` を評価。`counter = times; loop { counter -= 1; if !(counter >= 0) break; body; yield_loop }` — **最後の反復の後にも yield する**。`repeat 0` や負数は yield なしで抜ける。`∞` は無限 |
| repeat until <c> | 毎回 `B(c)` を評価。false の間 body → yield_loop |
| while <c>（隠しブロック `control_while`） | 同上で真の間 |
| forever | body → yield_loop を無限に |
| for each（隠し） | `stackFrame.index < Number(VALUE)` の間 `index++`, 変数 = index（number） |
| if / if else | `B(c)` |
| wait (s) | 初回のみ `duration = max(0, 1000*N(s))` を確定し、**redraw を要求**し、必ず1回 yield。以後、経過 < duration の間 yield。`wait 0` でも1 tick 待つ |
| wait until <c> | `B(c)` が false なら yield（warp 中は即再評価） |
| stop all | 全スレッド停止・全クローン削除 |
| stop this script | スレッド終了。**手続き内なら手続きからの return**（要検証: `thread.stopThisScript` の挙動） |
| stop other scripts in sprite | 同一ターゲットの他スレッドを停止 |
| create clone of | §10.8 |
| delete this clone | オリジナルなら何もしない |
| counter 系（隠し） | グローバルカウンタ |

### 7.4 手続き（カスタムブロック）
- 引数は**生の値**で渡す（キャストしない）。引数レポーター（`argument_reporter_string_number`）は値が無い（定義外で使用等）とき `0`、`argument_reporter_boolean` は `false`（要検証: getParam が null の場合）。
- 呼び出し側で入力が無い引数はミューテーションの `argumentdefaults` を使う（文字列引数は `""`、ブール引数は `"false"` という**文字列**になり得る。要検証）。
- 同名引数が複数ある場合は最後の値が勝つ（要検証）。
- 定義が見つからない呼び出しは何もしない。
- **再帰呼び出し**: 非 warp スレッドでは再帰呼び出し時に yield する（`isRecursiveCall`）。

### 7.5 入力の欠落
- 空のブール入力スロット（`if <>` 等）は `false`。
- 空のブールスロットを `B()` 以外に渡す経路は通常存在しない。存在した場合はバニラの `undefined` 相当を `Value::Str("undefined")`... ではなく、まず差分テストで確認（要検証）。

---

## 8. 型システムと型推論

### 8.1 方針
TurboWarp は**スクリプト単位**のフロー解析で、yield のたびに変数型を `ANY` に戻す。scrust は AOT でプロジェクト全体が見えるため、**2 層**で解析する:

- **層1: 格納型（storage type）** — 各変数・リスト・手続き引数について「プロジェクト全体で書き込まれ得る値の型の和」。これが Rust の**格納表現**を決める（`f64` / `bool` / `JsStr` / `Value`）。
- **層2: フロー型（flow type）** — 各スクリプト・手続き内でのデータフロー解析（TurboWarp の TypeState 相当）。yield 点では型を `ANY` ではなく**層1の格納型**に戻す（他スレッドは格納型の範囲内でしか書き込めないため）。

### 8.2 型格子（ビットセット）
TurboWarp の `InputType` を拡張する。`u16`/`u32` ビットフラグ:

```rust
bitflags! {
    pub struct Ty: u32 {
        // number
        const POS_INF   = 1 << 0;
        const POS_INT   = 1 << 1;   // 自然数（1 以上の整数、有限）
        const POS_FRACT = 1 << 2;   // 正の非整数
        const ZERO      = 1 << 3;   // +0
        const NEG_ZERO  = 1 << 4;   // -0
        const NEG_INT   = 1 << 5;
        const NEG_FRACT = 1 << 6;
        const NEG_INF   = 1 << 7;
        const NAN       = 1 << 8;
        // string（scrust で細分化）
        const STR_CANON = 1 << 9;   // 正準数値文字列: String(Number(s)) === s かつ /^-?\d+(\.\d+)?$/
        const STR_NUM   = 1 << 10;  // その他の数値解釈可能な文字列（"1.0", " 5", "0x10", "1e3", "-0", "Infinity"）
        const STR_NAN   = 1 << 11;  // 数値でない文字列（"", 空白のみ, "abc"）※空白のみは Number=0 だが compare では NaN 扱い
        const STR_BOOL  = 1 << 12;  // "true"/"false"（大文字小文字問わず）。B() が型だけで決まる
        // boolean
        const BOOL_T    = 1 << 13;
        const BOOL_F    = 1 << 14;

        const NUMBER = Self::POS_INF.bits | ... | Self::NEG_INF.bits; // NaN を除く
        const NUMBER_OR_NAN = Self::NUMBER.bits | Self::NAN.bits;
        const STRING = Self::STR_CANON.bits | Self::STR_NUM.bits | Self::STR_NAN.bits | Self::STR_BOOL.bits;
        const BOOLEAN = Self::BOOL_T.bits | Self::BOOL_F.bits;
        const ANY = Self::NUMBER_OR_NAN.bits | Self::STRING.bits | Self::BOOLEAN.bits;
    }
}
```
定数は値そのものから正確な型を付ける（`"10"` → `STR_CANON`, `"1.0"` → `STR_NUM`, `10.0` → `POS_INT`）。定数は型に加えて**値自体**も IR に保持し、定数畳み込みに使う。

### 8.3 演算子の結果型
- 四則演算: TurboWarp の `iroptimizer.js` の規則（入力の符号・整数性・無限・ゼロから、結果の可能性を求める）と同等のものを実装。**オペランドは先に `to_number` の結果型（NaN を除去、文字列は数値型へ写像）に変換してから**適用する。
  - 文字列 → 数値の型写像: `STR_CANON` → その値の符号・整数性が分かれば精密に、不明なら `NUMBER`。`STR_NUM` → `NUMBER`（`"Infinity"` を含み得る）。`STR_NAN` → `ZERO`。`STR_BOOL` → `ZERO`。`BOOL_T` → `POS_INT`、`BOOL_F` → `ZERO`。
- `round`, `floor`, `ceiling` → 整数性を付与。`abs` → 非負。`length`, `item # of`, `length of list` → `POS_INT | ZERO`。
- `join`, `letter of`, `item of list` 以外の文字列系 → `STRING`（細分不明）。`letter of` は `STR_CANON | STR_NAN`（1文字。`"0"`〜`"9"` は正準）。
- 比較・論理 → `BOOLEAN`。
- `item (i) of list` → リストの格納型 ∪ `STR_NAN`（無効インデックスで `""`）。
- `answer` → `STRING`。`username` → `STRING`。`x position` 等 → `NUMBER`（§11 の limitPrecision 後）。`timer` → `POS | ZERO`。`days since 2000` → `NUMBER`。
- 手続き引数レポーター → 層1で求めた**引数の格納型**（全呼び出し地点の実引数型の和 ∪ デフォルト値の型 ∪ 「定義外使用なら `ZERO`」）。

### 8.4 層1（格納型）の計算
1. 全スクリプト・全手続きを走査し、各変数への書き込み（`set`, `change`（→NUMBER）, `for each`（→POS_INT）, 初期値, 隠しブロック）の値の型を集める。
2. 値の型は層2の推論結果に依存するため、**全体を不動点反復**で解く（初期値: 全変数 = 初期値の型のみ → 単調増加 → 収束。格子は有限なので停止する）。
3. **外部書き込み**も和に加える:
   - スライダーモニター → `NUMBER`。
   - リストモニターの手入力（プレイヤーで編集可能にする場合）→ `STRING` 全体。既定ではモニター編集を**無効**にし、`--editable-monitors` で有効化。
   - クラウド変数 → `STRING`（サーバ連携する場合）。
   - インポート（リストの右クリック import）は非対応。
4. `sensing_of`（他スプライトの変数読み取り）は読み取りのみなので格納型に影響しないが、§8.6 の「種別敏感な読み取り」の判定には含める。プロパティ名が動的（レポーターが刺さっている）場合は、そのスプライトの**全変数**を「種別敏感に読まれ得る」とみなす。

### 8.5 層2（フロー型）
- TurboWarp の `TypeState` と同じ構造: 変数ID → 型。`if/else` の合流は和、ループは不動点、`stop this script` は出口状態に加算。
- **yield 点**（ループ末尾の yield_loop、`wait`、`wait until`、`broadcast and wait`、`ask and wait`、yield し得る手続き呼び出し、`glide` 等）の直後は、全変数の型を**層1の格納型**にリセット（TurboWarp は ANY）。
- 手続き呼び出しは手続きごとの出口状態をキャッシュ（yield する手続きは overwrite、しない手続きは after 合成。TurboWarp と同じ）。
- **ローカル変数化**: スプライトローカル変数でも他スクリプトが読み書きし得るため、原則メモリ上の格納場所に毎回読み書きする。ただし「yield を挟まない区間」では Rust のローカル変数にキャッシュしてよい（他スレッドは割り込めない）。区間末尾・yield 直前・手続き呼び出し直前（その手続きが当該変数に触れるなら）に書き戻す。

### 8.6 文字列/数値の区別を消してよい条件（最重要）
Scratch では `"10"`（文字列）と `10`（数値）はほとんどの演算で区別できないが、**完全には区別できなくならない**。以下が「種別敏感（kind-sensitive）シンク」:

| シンク | 文字列と数値で何が変わるか |
|---|---|
| `switch costume to (x)` / `switch backdrop to (x)` / `switch backdrop to and wait` | 数値なら番号、文字列ならまず**名前**で検索（コスチューム名 `"2"` がある場合に挙動が変わる） |
| `play sound (x)` / `start sound (x)` | 同様に数値なら番号、文字列ならまず名前 |
| `pick random` の `is_int` | `"1.0"` vs `1`（ただし `STR_CANON` は `.` を含まず整数なら `is_int` 一致、小数なら両方 false… 正準 `"1.5"` は false、`1.5` も false で一致） |
| `listcontents`（バニラ） | `["1","2"]` → `"12"`、`[1,2]` → `"1 2"` |
| リストの `contains` の同一性判定 | 結果は compare で補われるので**観測不能**（シンクではない） |
| 拡張・未知のブロック | 安全側としてシンク扱い |

**規則**:
- 型 `STR_CANON` の値を `f64` として表現（＝数値化）してよいのは、その値（またはそれを格納した変数・リスト）が**どの種別敏感シンクにも到達しない**ことを、プロジェクト全体のデータフローで証明できた場合に限る。
- `STR_CANON` の定義に `-0` は含まれない（`"0"` は `+0`。`-0` の数値は `"0"` に化けるので逆変換も禁止）。指数表記・`"1.0"` 等は `STR_NUM` であり、決して数値化しない（`length`, `join`, `letter of` で区別できる）。
- 種別敏感シンクの直前に置く値は、元の型タグ（`Value::Str` / `Value::Num`）を保持したまま渡すこと。
- これにより、`set score to 0`（ブロック定数は sb3 上で文字列 `"0"`）のような**極めて一般的なパターン**でも、`score` を `f64` にアンボックスできる（コスチューム切り替えに `score` を使っていなければ）。

### 8.7 格納表現の決定
| 層1の型 | Rust 格納型 |
|---|---|
| ⊆ `NUMBER_OR_NAN`（§8.6 で数値化できる `STR_CANON` を含む） | `f64` |
| ⊆ `BOOLEAN` | `bool` |
| ⊆ `STRING` | `JsStr` |
| それ以外 | `Value` |

リストも同様に要素型で `Vec<f64>` / `Vec<JsStr>` / `Vec<Value>` / `Vec<bool>` を選ぶ（要素型の和で決定）。リストの `listcontents` が使われ、かつバニラ互換モードのときは、`Vec<f64>` に `STR_CANON` を入れてはならない。

### 8.8 型推論レポート
`scrust check --types` は全変数・リスト・引数の格納型と、`Value` に落ちた理由（どの書き込みが原因か）を出力する。性能チューニングとデバッグの両方に使う。

---

## 9. Rust コードへの写像

### 9.1 キャスト関数（`scrust-value`）
すべて `#[inline]`。型別のオーバーロードを持たせ、静的に分かる場合は分岐を消す:
```rust
pub trait ToNum { fn to_num(&self) -> f64; }
impl ToNum for f64   { #[inline(always)] fn to_num(&self) -> f64 { if self.is_nan() { 0.0 } else { *self } } }
impl ToNum for bool  { #[inline(always)] fn to_num(&self) -> f64 { *self as u8 as f64 } }
impl ToNum for JsStr { fn to_num(&self) -> f64 { /* §5.2 */ } }
impl ToNum for Value { fn to_num(&self) -> f64 { match self { ... } } }
```
型解析で NaN を含まない `f64` と分かれば `to_num` 自体を省略（TurboWarp の cast 除去と同じ）。

### 9.2 生成コード例
入力:
```
when flag clicked
set [i] to (0)
repeat (10)
  change [i] by (1)
  say (join [i=] (i))
end
```
`i` の格納型は `f64`（"0" は STR_CANON、種別敏感シンク無し）。生成（概念図）:
```rust
pub async fn sprite1_script0(me: TargetRef) {
    { let w = world(); w.globals.v_i = 0.0; }
    let mut counter: f64 = 10.0;              // js_round(N("10")) を定数畳み込み
    loop {
        counter -= 1.0;
        if !(counter >= 0.0) { break; }
        {
            let w = world();
            w.globals.v_i += 1.0;             // N(f64 非NaN) は省略
            let s = JsStr::lit("i=").concat(&js_num_to_str(w.globals.v_i));
            w.say(me, Value::Str(s));
        }
        yield_loop().await;                   // 非 warp: 必ず yield
    }
}
```
`counter` は値域が `[−1, 10]` と証明できるので `i32` に置換してよい。

### 9.3 比較の高速パス
| 左右の型 | 生成コード |
|---|---|
| 両方 NaN を含まない数値 | `a == b` / `a < b` / `a > b` |
| 片方が定数の `STR_NAN`（例: `"abc"`）、他方が数値 | 文字列比較に確定 → `to_string(n).to_lower() cmp "abc"` |
| 両方文字列で、片方が `STR_NAN` 定数 | 小文字化済み定数との文字列比較 |
| その他 | `scrust_value::compare(&a, &b)` |

### 9.4 整数への特殊化
許可される条件: 値域解析で `|x| < 2^31` かつ整数かつ `-0` が生じない（加算・減算のみ等）ことを証明できた**ローカル**値（ループカウンタ、インデックス計算）。格納変数は常に `f64`。

---

## 10. 実行モデル（スケジューラ）

バニラの `Runtime._step` → `Sequencer.stepThreads` → `stepThread` を**意味的に**再現する。観測可能なのは「フレーム内で各スレッドがどこまで進むか」と「スレッドの実行順」。

### 10.1 フレーム（tick）ループ
```
every frame (既定 60 fps / --fps 30 で compatibility mode):
    threads から isKilled を除去
    edge-activated hat（when [timer > ...], when [loudness > ...] 等）を評価して起動
    redraw_requested = false
    monitor 用スレッド処理
    stepThreads():
        WORK_TIME = 0.75 * frame_interval
        current_msecs = now()                       // timer ブロックはこの値を使う（tick 内で不変）
        ran_first_tick = false
        while threads 非空 && 実行中スレッドあり && elapsed < WORK_TIME
              && (turbo || !redraw_requested):
            for i in 0..threads.len():               // ※len は毎回再評価（途中で追加されたスレッドも同じ周回で走る）
                t = threads[i]
                if done: continue
                if t.status == YIELD_TICK && !ran_first_tick: t.status = RUNNING
                if t.status ∈ {RUNNING, YIELD}: step(t)   // 次の yield まで進める
                ...
            ran_first_tick = true
            完了スレッドを除去（順序維持）
    render
```
**要点**: 非 turbo では、どのスレッドも描画変化（redraw 要求）を起こさなければ、同じフレーム内で全スレッドを**何周も**回す（最大 フレーム時間の 75%）。描画を変えるブロックが1つでも実行されるとその周回で打ち切り。これを外すと、ループ回数あたりのフレーム数が変わり `timer` を使うプロジェクトが壊れる。

### 10.2 redraw を要求するブロック
- 表示中スプライトの位置・向き・大きさ・コスチューム・エフェクト・表示/非表示・レイヤーの変更（`RenderedTarget` の各 setter で `if (visible) requestRedraw()`。非表示スプライトの移動は要求しない点に注意。要検証: 各 setter の条件を全列挙）
- ペンのすべての描画、`clear`、`stamp`
- `say`/`think`
- `wait`（初回）
- ステージの背景切り替え
実装: `World::request_redraw()` を各 setter に組み込み、リストを `docs/decisions/redraw.md` に作る。

### 10.3 スレッドと yield の種類
| 状態 | 意味 | scrust |
|---|---|---|
| YIELD | この周回は終わり、次の周回（同フレームでも可）で再開 | `yield_now().await` |
| YIELD_TICK | 次の**フレーム**まで再開しない | `yield_tick().await` |
| PROMISE_WAIT | 外部完了待ち（音の再生完了、ask の入力） | `wait_external(handle).await` |
| ループ末尾 | 非 warp: YIELD 相当。warp: warp タイマー超過時のみ | `yield_loop().await` |

### 10.4 warp（画面を再描画せずに実行）
- warp は呼び出し元から**継承**される（warp 手続きから呼ばれた非 warp 手続きも warp で動く）。
- warp 中はループ末尾で yield しない。`wait`/`wait until`/`ask` 等の明示的 yield は warp 中でも起きる（ただし YIELD は warp 中なら即再実行されるため、実質「待つ」のは YIELD_TICK・PROMISE_WAIT 系と時間経過系のみ。`wait 1` は warp 中でも 1 秒待つ＝ビジーループ）。
- `--warp-timer` 有効時: warp 開始から 500ms 超過でループ末尾/再帰呼び出しで yield。チェックは 100 反復に1回程度に間引いてよい（TurboWarp の isStuck と同様）。

### 10.5 コード生成の形（async）
- 各スクリプト = `async fn`。シングルスレッド executor（自作、`scrust-runtime::sched`）がスレッドリストを管理し、§10.1 の順で `poll` する。各スレッドの Future は `Pin<Box<dyn Future<Output=()>>>`。
- 各手続きは **(warp / 非 warp) × (yield し得る / しない)** でバリアント化:
  - yield し得ないバリアント（warp 中で `wait` 等を含まない、かつ warp タイマー無効）は**普通の `fn`**（同期・高速）。`&mut World` を引数で受け取る。
  - yield し得るバリアントは `async fn`。再帰は `Box::pin`。
- yield 解析は呼び出しグラフ上の不動点（再帰を含む SCC は保守的に）。

### 10.6 World へのアクセス（借用規則）
- 全ゲーム状態は `World` に集約。executor は poll の間だけ `World` へのポインタを thread-local（wasm ではグローバル）にセットする（TurboWarp の `globalState.thread` に相当）。
- **不変条件**: (1) `async fn` 内で得た `&mut World` を `.await` を跨いで保持しない。(2) 同期関数には `&mut World` を**引数で渡し**、内部で `world()` を再取得しない。生成器はこの2規則を構文的に保証するコードしか出さない。
- 実装は `WorldCell`: debug ビルドでは RefCell 風の借用フラグで違反を panic、release では無検査。Miri でランタイムのテストを回す。

### 10.7 ハット・イベント・実行順
- **ターゲットの実行順**: `executableTargets` は描画レイヤー順と同期しており、**最前面のスプライトのスクリプトから順に**起動し、ステージは最後。レイヤー操作（go to front、go forward、クローン生成時の goBehindOther 等）のたびに更新する。
- **ターゲット内のスクリプト順**: sb3 の `blocks` オブジェクト内でのトップレベルブロックの出現順。**JS のオブジェクトキー順序**（配列インデックス風のキー `"0"`〜`"4294967294"` は数値昇順で先頭、他は挿入順）を再現すること。`serde_json` は `preserve_order` 機能を使い、読み込み後に JS 順に並べ替える。
- `startHats`: マッチしたハットごとに
  - `restartExistingThreads`（`when I receive`、`when flag clicked`、`when this sprite clicked`、`when backdrop switches to`、`when key pressed`?（要検証: ハットごとのフラグを全列挙））が真なら、同じ (ターゲット, トップブロック) の既存スレッドを**リスト内の同じ位置で**作り直す。
  - 偽（`when I start as a clone` 等）なら、実行中なら起動しない。
  - 新規スレッドはリスト末尾に追加。**起動時にハットブロック自体はその場で実行され、次のブロックから始まる**。
- `broadcast`: 起動のみ。`broadcast and wait`: 初回に起動したスレッド群を保存し、0本ならそのまま続行。いずれかがまだ生きていれば、全部が「待機中（PROMISE_WAIT / YIELD_TICK / 終了）」なら YIELD_TICK、そうでなければ YIELD。
- ブロードキャスト名の照合は**大文字小文字を無視**（`startHats` が `toUpperCase` で比較）。動的なブロードキャスト名（レポーターを刺した場合）も `S().to_upper()` で照合。
- `green flag`: stop all → タイマーリセット → 起動。
- edge-activated ハット（`when [timer/loudness] > (x)`）: 毎フレーム評価し、偽→真の遷移でのみ起動。

### 10.8 クローン
- 上限 300（`MAX_CLONES`）。超過時は作らない。
- クローンは元の状態（位置、向き、コスチューム、サイズ、エフェクト、表示、ドラッグ可否、回転方式、ペン状態、音量、**ローカル変数とリストの値のコピー**）を複製し、元ターゲットの**直後ろのレイヤー**に入る。
- 生成直後に `when I start as a clone` を起動。
- 表現: スプライト（原型）ごとに生成された構造体 `struct Sprite3Locals { v_speed: f64, l_items: Vec<Value>, ... }` と共通部 `SpriteState` を持つインスタンスを、スプライト種別ごとのアリーナ（`slotmap`）に格納。`TargetRef` は (種別, 世代付きインデックス)。削除済みクローンを参照するスレッドは停止扱い。
- `stop all` で全クローン削除。

### 10.9 時間
- `timer` は `runtime.current_msecs`（tick 開始時に更新）基準 → **1 tick 内で値は変わらない**。
- `days since 2000` は現在時刻を直接使う。
- `current [year/month/...]` はローカル時刻。
- 決定的モード: 仮想時計（フレームごとに `1000/fps` ms 進む）、乱数シード固定（`xorshift128+` 等、`Math.random` 相当の [0,1) 一様）、WORK_TIME 判定は「周回数上限」に置き換え可能にする（差分テストでは scratch-vm 側も同じ決定化を行う。§15）。

---

## 11. ブロックごとの値の細部（型に関わるもの）

- `x position` / `y position` / `direction`: `limitPrecision`（最寄り整数との差が `1e-9` 未満なら整数に丸める）を通した値を返す。
- `move (n) steps`: `rad = (90 - direction) * PI / 180; x += n*cos(rad); y += n*sin(rad)`（`libm`。丸め無し）→ フェンシング（ステージ外に出過ぎない）を適用。
- `point in direction`: `wrapClamp(dir, -179, 180)`。
- `set size`: コスチュームとステージの大きさから決まる最小/最大でクランプ（scratch-render の計算を再現。要検証）。
- `switch costume to (x)`（§8.6）:
  - 数値: `setCostume(x - 1)` → 内部で `js_round` → `±∞`/NaN は 0 → `wrapClamp(0, n-1)`。
  - 文字列: 名前一致を優先 → `"next costume"` / `"previous costume"` → 数値として解釈可能かつ空白のみでなければ番号として扱う。
  - バニラの `looks._setCostume` / `_setBackdrop` を読んで分岐順を正確に写すこと（要検証）。
- `costume [number/name]`, `backdrop [number/name]`: 番号は number、名前は string。
- `size`: `js_round(size)`（要検証）。
- `answer`: 常に string。`ask` は入力確定まで PROMISE_WAIT。
- `(attribute) of (sprite)`: 変数を返すときは**格納されている生の値**（型保持）。存在しなければ `0`。
- `key (k) pressed?`: キー名の照合規則（`"space"`, `"any"`, 1文字は大文字小文字無視等）を scratch-vm の keyboard.js から写す。
- `say (x)`: `S(x)`。数値の表示は §5.3。空文字列で吹き出しを消す。
- `change [effect] by`, `set volume`: 数値クランプ規則を写す。

---

## 12. sb3 読み込み

- zip を開き `project.json` を読む。`serde_json` は `preserve_order` と `float_roundtrip` を有効化。
- **数値の JSON 値は number、文字列は string のまま** `Value` に変換（ブロック定数の多くは文字列）。
- 入力配列の形式: `[shadowKind(1|2|3), input, shadow?]`。`input` は ブロックID文字列 or プリミティブ配列:
  | code | 意味 | 値 |
  |---|---|---|
  | 4 | math_number | `[4, value]` |
  | 5 | math_positive_number | |
  | 6 | math_whole_number | |
  | 7 | math_integer | |
  | 8 | math_angle | |
  | 9 | colour_picker | `"#rrggbb"` |
  | 10 | text | |
  | 11 | event_broadcast_menu | `[11, name, id]` |
  | 12 | data_variable | `[12, name, id, x?, y?]`（トップレベル時は座標付き） |
  | 13 | data_listcontents | `[13, name, id, ...]` |
  - 数値系フィールドでも値は**文字列のまま**（`"10"`, `""`, `"abc"` もあり得る）。`math_number` の空 `""` は `N("")=0` だが `S` は `""`。
- shadowKind 3（obscured）: レポーターが刺さり、下にシャドウがある。評価はレポーター。
- 変数: `[name, value]` または `[name, value, true]`（クラウド）。リスト: `[name, [items...]]`。
- 色入力（`touching color`, `pen color`）: `"#rrggbb"` 文字列 or 数値（`0xAARRGGBB` 相当の10進）→ `Cast.toRgbColorObject` を写す。
- ターゲットは `layerOrder` でソートしてレイヤー順を復元。
- 未対応の opcode・拡張は `scrust check` で報告し、ビルドは `--allow-unsupported` の時のみ（そのブロックは no-op、レポーターは `""`）。
- 壊れたプロジェクト（参照先の無いブロックID、親子不整合、`next` の循環）に耐えること。循環は検出して切る。

---

## 13. ランタイム（描画・音・入力）

型の核心ではないが、互換性に必要な要件:

- **描画**: `wgpu`（native と WebGPU/WebGL2）。ステージ 480×360、高 DPI 対応。
  - SVG: `usvg` + `resvg` で、スプライトの表示サイズに応じたミップレベル（scratch-render と同様に 2 の冪スケール）でラスタライズ。Scratch のフォント（Sans Serif, Serif, Handwriting, Marker, Curly, Pixel 等。OFL ライセンスのものを同梱）を `fontdb` に登録。
  - ビットマップ: `bitmapResolution`（通常 2）を考慮。
  - グラフィックエフェクト（color, fisheye, whirl, pixelate, mosaic, brightness, ghost）を scratch-render のシェーダと同じ式で実装。
- **当たり判定**: `touching sprite/edge/mouse`, `touching color`, `color touching color` は scratch-render の CPU 経路（Silhouette のサンプリング、エフェクト適用後の座標変換）と**同じサンプリング規則**で CPU 実装。GPU 読み戻しはしない。
- **ペン**: ステージサイズのオフスクリーンテクスチャ。線の太さ・端の形・色（HSV + 透明度、レガシーの hue/shade）を scratch3_pen の式で。
- **吹き出し・ask・モニター**: 自前描画。変数モニターの数値表示は scratch-gui の整形規則（要検証: 小数点以下桁数の制限有無）。
- **音**: `cpal` + 自前ミキサー。WAV（ADPCM 含む）と MP3 をデコード（`symphonia`）。ピッチ/パン/音量エフェクト、`play until done`（PROMISE_WAIT）、`stop all sounds`。
- **入力**: キーボード（Scratch のキー名への写像）、マウス（ステージ座標、クリックでのスプライト選択は最前面からのピクセル単位ヒットテスト）、ドラッグ可能スプライト。
- **web**: `wasm-bindgen` + 単一 HTML。音声は Web Audio。

---

## 14. 互換モードとオプション

| オプション | 既定 | 内容 |
|---|---|---|
| `--compat=default` | ✓ | バニラ準拠。warp タイマー無効（TurboWarp 既定と同じ）。 |
| `--compat=strict` | | warp タイマー有効、モニター編集等も含めバニラに最大限一致。差分テストはこのモード。 |
| `--fps N` | 60 ※ | バニラ通常は 30 fps 互換モードがプロジェクト既定でない点に注意（scratch-gui 既定は 30 TPS? 要検証）。プロジェクトに TurboWarp の設定コメント（`// _twconfig_`）があれば読む。 |
| `--turbo` | off | redraw 要求で周回を打ち切らない。 |
| `--interpolate` | off | TurboWarp 互換の補間（任意、Phase 6）。 |
| `--deterministic --seed N` | off | §10.9 |

※ fps 既定値は着手時に scratch-gui の既定を確認して決める（要検証）。

---

## 15. テスト戦略

### 15.1 差分テスト（最重要）
- `conformance/harness/`: Node.js で scratch-vm（固定コミット、ヘッドレス、レンダラ無しまたは scratch-render を headless-gl で）を動かし、各フレーム終了時に以下を JSON Lines で出力する `trace` スクリプト:
  - 全ターゲットの全変数・リスト（**型タグ付き**: `{"t":"num","v":1}` / `{"t":"str","v":"1"}` / `{"t":"bool","v":true}`。数値は `-0`, `NaN`, `±Infinity` を区別できる表現で）
  - `say`/`think` の内容、x/y/direction/costume/size/visible/layer
  - 実行中スレッド数、そのフレームでの周回数
- 両者を `--deterministic` で同一条件（同じ乱数列、同じ仮想時計、WORK_TIME を周回上限に置換）にして実行し、トレースを比較する。乱数は scratch-vm 側の `Math.random` をシード付き実装に差し替える。
- `scrust trace` が同形式を出力する。

### 15.2 式ファザー
- ランダムな式木（演算子ブロック・リスト操作・キャスト境界値の定数）を生成 → sb3 を合成 → 両実装で評価 → 型タグ付きで完全一致を要求。
- 境界値辞書: `""`, `" "`, `"\t"`, `"\u00a0"`, `"0"`, `"-0"`, `"0.0"`, `"00"`, `"1."`, `".5"`, `"1e3"`, `"1E+3"`, `"0x1F"`, `"0X1f"`, `"0b101"`, `"0o17"`, `"-0x1"`, `"Infinity"`, `"-Infinity"`, `"infinity"`, `"NaN"`, `"true"`, `"FALSE"`, `"abc"`, `"ABC"`, `"İ"`, `"Σ"`, `"😀"`, 孤立サロゲート, `0`, `-0`, `0.5`, `-0.5`, `2.5`, `-2.5`, `1e21`, `1e-7`, `2**53`, `2**53+2`, `NaN`, `±Infinity`, `5e-324`, `true`, `false`。
- 型推論の健全性テスト: 推論で `f64` 化したプログラムと、全変数を `Value` に強制したプログラム（`--no-unbox`）の出力が一致すること。

### 15.3 ユニットテスト必須表（抜粋。すべて実装すること）
| 式 | 期待値（型付き） |
|---|---|
| `"" + 1` | num 1 |
| `"abc" * 2` | num 0 |
| `" 12 " + 0` | num 12 |
| `"0x10" + 0` | num 16 |
| `"-0x10" + 0` | num 0 |
| `"1_0" + 0` | num 0 |
| `"Infinity" + 0` | num Infinity |
| `"infinity" + 0` | num 0 |
| `0 / 0` | num NaN（say すると "NaN"） |
| `(0/0) * 1` | num 0 |
| `1 / 0` | num Infinity |
| `0.1 + 0.2` | num 0.30000000000000004 |
| `join 1e21 ""`（1e21 は計算結果） | str "1e+21" |
| `join (0 * -1) ""` | str "0" |
| `1 / (0 * -1)` | num -Infinity |
| `round -2.5` | num -2 |
| `round 2.5` | num 3 |
| `-1 mod 3` | num 2 |
| `5 mod -3` | num -1 |
| `log 1000` | num 2.9999999999999996 |
| `sin 180` | num 0 |
| `cos 90` | num 0 |
| `tan 90` | num Infinity |
| `tan -90` | num -Infinity |
| `length "😀"` | num 2 |
| `letter 1.9 of "abc"` | str "a" |
| `letter 0.5 of "abc"` | str "" |
| `"abc" contains ""` | bool true |
| `"Apple" contains "PP"` | bool true |
| `NaN = NaN` | bool true |
| `"" = 0` | bool false |
| `"1.0" = 1` | bool true |
| `"abc" = "ABC"` | bool true |
| `"10" < "9"` | bool false |
| `"10" < "9a"` | bool true |
| `true = "TRUE"` | bool true |
| `<not "false">` | bool true |
| `<not "0.0">` | bool false |
| `item 1.9 of [a,b]` | str "a" |
| `item "last" of []` | str "" |
| `item # of 123 in [4,"123"]` | num 2 |
| listcontents `["a","b"]` | str "ab" |
| listcontents `["a",1]`（1 は数値） | str "a 1" |
| listcontents `["ab","c"]` | str "ab c" |
| `repeat 2.5` | 3 回 |
| `repeat "abc"` | 0 回 |
| `random 1 to 1` | num 1 |
| `random "1.0" to 2` | 小数あり |

### 15.4 実プロジェクト
- 公開されている Scratch 3 プロジェクトから、ライセンス上問題の無い代表的なもの（または自作の互換性テストプロジェクト集）を `conformance/projects/` に置き、N フレームのトレース一致を CI で確認。
- 性能ベンチ: 典型的な重いプロジェクト（ペンでのレイトレ、大量クローン、リスト処理）で TurboWarp との実行時間比較。

---

## 16. フェーズ計画と Done 定義

### Phase 0 — 基盤（scrust-value）
- `Value`, `JsStr`（ロープ含む）, §5 の全アルゴリズム, §6 の演算子関数。
- **Done**: §15.3 の表が全て通る。`js_string_to_number`・`js_num_to_str` を Node の `Number()`/`String()` と 1,000 万件のランダム入力で突き合わせて不一致 0。

### Phase 1 — フロントエンドとヘッドレス実行
- sb3 パーサ、IR、名前解決、定数畳み込み、**全値を `Value` で扱う素朴なコード生成**（型推論なし）。
- スケジューラ（§10 全体、redraw 要求の擬似実装）、イベント、手続き、クローン（描画なし）、変数・リスト、`say` をトレース出力。
- 差分テストハーネス（§15.1）と式ファザー（§15.2）。
- **Done**: 制御・演算・データ・イベント・手続きのみからなるテストプロジェクト群で、決定的モードのトレースが scratch-vm と完全一致。

### Phase 2 — 型推論とアンボックス化
- §8 全体、§9 の高速パス、手続きバリアント、同期関数化。
- **Done**: `--no-unbox` との出力一致（健全性）。ベンチで Phase 1 比 5 倍以上。`scrust check --types` 出力。

### Phase 3 — 描画と入力
- wgpu レンダラ、SVG/ビットマップ、Motion/Looks 全ブロック、フェンシング、レイヤー、吹き出し、ask、モニター、キーボード・マウス、当たり判定（CPU）。
- **Done**: Looks/Motion/Sensing を使うテストプロジェクトで x/y/costume/size/layer のトレース一致。目視比較スクリーンショットテスト（許容差付き）。

### Phase 4 — ペンと音
- **Done**: ペン描画のピクセル比較（許容差付き）、音ブロックのタイミング（`play until done` の待機時間）一致。

### Phase 5 — wasm と配布
- **Done**: 同一プロジェクトがブラウザで動作。単一 HTML 出力オプション。

### Phase 6 — 任意
- music 拡張、クラウド変数、補間、TurboWarp 設定コメント、sb2 読み込み、カスタムレポーター（TurboWarp 互換の return）。

---

## 17. 落とし穴チェックリスト（実装中に毎回確認）

- [ ] `f64::from_str` や `format!("{}", f64)` を値変換に直接使っていないか
- [ ] `f64::round` を使っていないか（`js_round` を使う）
- [ ] 文字列長・インデックスを UTF-8 バイトや `char` で数えていないか（UTF-16 単位）
- [ ] ブロック定数 `"10"` を無条件に数値化していないか（§8.6）
- [ ] `-0` を `"-0"` と表示していないか／`"-0"` を `+0` にしていないか
- [ ] `NaN` を数値演算に入れる前に 0 にしているか、変数格納時には NaN のまま保持しているか
- [ ] `NaN = NaN` が true になるか
- [ ] `repeat` の回数を初回だけ評価しているか、最後の反復後にも yield しているか
- [ ] `wait 0` が1 tick 待つか、redraw を要求するか
- [ ] フレーム内で複数周回するか（redraw 要求が無い場合）
- [ ] 途中で追加されたスレッドが同じ周回で実行されるか、restart が同じ位置に入るか
- [ ] スクリプトの起動順（レイヤー前面から、ステージ最後、ブロックの JSON キー順）
- [ ] `timer` が tick 内で一定か
- [ ] ブロードキャスト名の大文字小文字無視
- [ ] `listcontents` の単一文字判定が「型 string かつ長さ1」か
- [ ] `log` を `ln(x)/LN10` で計算しているか
- [ ] sin/cos/tan の `toFixed(10)` 丸め
- [ ] `.await` を跨いで `&mut World` を保持していないか
- [ ] JSON のキー順（整数風キー先頭）を再現しているか

---

## 18. 参照ソース（読むべき順）

scratch-vm（`scratchfoundation/scratch-vm`, `develop`）:
1. `src/util/cast.js` — キャスト・比較・リストインデックス
2. `src/blocks/scratch3_operators.js` — 演算子
3. `src/blocks/scratch3_data.js` — 変数・リスト（上限 200000、listcontents）
4. `src/blocks/scratch3_control.js` — ループ・wait・クローン
5. `src/util/math-util.js` — `tan`, `wrapClamp`
6. `src/engine/sequencer.js` — tick 内の周回、WORK_TIME、warp、再帰時の yield
7. `src/engine/runtime.js` — `_step`, `startHats`, `_restartThread`, `allScriptsByOpcodeDo`, `MAX_CLONES`
8. `src/engine/thread.js`, `src/engine/execute.js` — スレッド状態、入力評価順
9. `src/blocks/scratch3_looks.js`, `scratch3_motion.js`, `scratch3_sensing.js`, `scratch3_event.js`, `scratch3_procedures.js`, `scratch3_sound.js`
10. `src/sprites/rendered-target.js` — setter と redraw 要求、costume 切り替え、クローン
11. `src/serialization/sb3.js` — 入力プリミティブ、変数・リストの形式
12. `src/io/keyboard.js`, `src/io/mouse.js`, `src/io/clock.js`

TurboWarp（`TurboWarp/scratch-vm`, `develop`）— 最適化の参考:
- `src/compiler/enums.js` — `InputType` ビット格子
- `src/compiler/iroptimizer.js` — `TypeState`、yield での型クリア、演算子の結果型規則
- `src/compiler/irgen.js`, `src/compiler/jsgen.js` — IR 化とキャスト挿入
- `src/compiler/jsexecute.js` — 比較の高速パス、`listContents` の変更点、`isStuck`

ECMAScript 仕様: StringToNumber（§7.1.4.1.1）、Number::toString（§6.1.6.1.20）、`Number.prototype.toFixed`、`String.prototype.toLowerCase`、`Math.round`。
