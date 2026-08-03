# microcms-tui

[English](README.md) | 日本語

Ratatui製のmicroCMS向けターミナルUIです。APIの選択、コンテンツの閲覧・編集、公開状態の変更、メンバー・メディア管理に対応しています。

## インストール

[GitHub Releases](https://github.com/istone-you/microcms-tui/releases)からmacOS、Linux、Windows向けバイナリをダウンロードできます。

ソースから実行・インストールする場合:

```sh
cargo run -- --service-id your-service --api-key your-api-key
cargo install --path .
```

## 設定

```sh
microcms-tui --service-id your-service --api-key your-api-key
microcms-tui --service-id your-service --api-key your-api-key --endpoint blogs --save-config
```

- CLI: `--service-id`, `--api-key`, `--endpoint`, `--save-config`
- 環境変数: `MICROCMS_SERVICE_ID`, `MICROCMS_API_KEY`
- 優先順位: CLI > 環境変数 > 設定ファイル
- `--endpoint`は初期選択のみ。起動後もTUI内で切り替え可能

検証環境などでAPIドメインが異なる場合は、次を指定できます。

- `MICROCMS_CONTENT_API_URL`
- `MICROCMS_MANAGEMENT_API_URL`

値はドメイン部分だけを指定してください。service IDと固定パス`/api/v1`は自動で付加されます。

```sh
MICROCMS_CONTENT_API_URL=https://microcms-staging.net
```

APIキーや認証情報をリポジトリへコミットしないでください。

## キーバインド

`?`で現在の画面に対応したヘルプを表示できます。`Ctrl-C`は常に終了です。

### 共通・ナビゲーション

| Key | Action |
| --- | --- |
| `?` | ヘルプ |
| `j`/`k`, `↑`/`↓` | 移動・スクロール |
| `Enter` | API選択・JSONプレビュー |
| `b`/`Esc` | 戻る・閉じる |
| `r` | 再読み込み |
| `n`/`p`, `PageDown`/`PageUp` | 次・前のページ |
| `M` | メンバー一覧 |
| `a` | メディア一覧 |

### コンテンツ

| Key | Action |
| --- | --- |
| `Space` | 一括操作対象をマーク |
| `c` / `C` | POST作成 / 下書き作成 |
| `u` / `U` | ID指定PUT作成 / 下書き作成 |
| `e` / `E` | PATCH編集 / `status=draft`で編集 |
| `d` | マーク中または現在のコンテンツを削除 |
| `P` / `D` | 公開 / 下書きへ変更 |
| `s` | 公開予約の表示・変更・解除 |
| `v` | 公開版・下書き版を表示 |
| `m` | 作成者を変更 |

通常の作成・編集、公開状態変更、削除は送信前に確認されます。JSON作成・編集には`$EDITOR`（未設定時は`vi`）を使用します。

### コンテンツクエリ

| Key | Query |
| --- | --- |
| `/` | キーワード検索 `q` |
| `f` / `o` | `filters` / `orders` |
| `l` | `fields`をスキーマから選択 |
| `z` | `depth`を選択 |
| `i` | カンマ区切りの`ids` |
| `K` | `draftKey` |
| `F` | `richEditorFormat` |
| `x` | クエリをすべて解除 |

### メディア

| Key | Action |
| --- | --- |
| `u` / `d` | アップロード / 削除 |
| `/` / `t` / `A` | ファイル名 / タグ / altで絞り込み |
| `I` | 画像のみを切り替え |
| `l` | ページ件数を設定 |
| `x` | 絞り込みを解除 |

マウスで行を選択でき、ホイールで一覧やJSONをスクロールできます。

## API権限

利用する機能に応じて、APIキーへ次の権限が必要です。

- Content API: GET / POST / PUT / PATCH / DELETE
- Management API: API・サービス情報、コンテンツ情報、公開状態、公開予約
- Members: 一覧・詳細取得、作成者変更
- Media: 取得、アップロード、削除

## 補足

- List APIは閲覧、検索、CRUD、一括操作、公開状態変更に対応
- Object APIはGETとJSONプレビューのみ対応
- 作成テンプレートと表示順はManagement APIのスキーマを使用
- APIエラーはJSONレスポンスをモーダルで表示
- ステータスドット: 公開=緑、下書き=シアン、予約=マゼンタ、終了=赤、不明=グレー
