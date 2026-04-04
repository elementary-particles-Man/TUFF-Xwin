# CLI Agent Task — Legal Hardening for OSS Publication

更新日: `2026-04-04`
対象 repository: `elementary-particles-Man/TUFF-Xwin`

## 前提

- 本PJは `MIT OR Apache-2.0` の OSS として無償公開する
- 目的は GUI 障害を kernel panic から切り離すための fault isolation 実装であり、第三者権利侵害を目的としない
- 実装優先の観点では、今やるべきことは機能削減ではなく **公開・配布に必要な法務証跡を repository に固定すること** である
- Vulkan 実装や新機能追加は今回スコープ外

---

## タスク 1: 依存ライセンス監査を CI に組み込む

### 目的
repo 自身のライセンスだけでなく、依存関係を含めて公開可能であることを追跡可能にする。

### 実施内容

1. `cargo-deny` を導入し、少なくとも `licenses` を CI で走らせる
2. `deny.toml` を追加し、当面の許可候補を明示する
   - `MIT`
   - `Apache-2.0`
   - `BSD-2-Clause`
   - `BSD-3-Clause`
   - `ISC`
   - `Unicode-DFS-2016`
   - `Zlib`
3. copyleft 系（`GPL-*`, `LGPL-*`, `AGPL-*` など）は、明示許可しない限り reject 方針
4. `.github/workflows/ci.yml` にライセンス監査 job か step を追加
5. 実行結果を `docs/status/DEPENDENCY_LICENSE_AUDIT_2026-04-04.md` に固定

### 完了条件
- 最新 main に対して依存ライセンス監査が CI で再現可能
- repo 内 doc で監査結果を追える

---

## タスク 2: 第三者依存の notices を固定

### 目的
第三者ソフトウェア由来の表示義務・配布義務に備える。

### 実施内容

1. `THIRD_PARTY_NOTICES.md` を新規作成
2. 少なくとも直依存について、以下を記録
   - crate 名
   - version
   - license
   - upstream URL
   - notice の要否
3. `NOTICE` が必要な依存が見つかった場合は、repo 直下 `NOTICE` を追加
4. `README.md` に `THIRD_PARTY_NOTICES.md` へのリンクを追加

### 完了条件
- 第三者依存の出所とライセンスが repo 内で追跡可能

---

## タスク 3: ログ / artifact / selection metadata の扱いを明文化

### 目的
プライバシー・業務情報・利用履歴メタデータに関する誤解と実務リスクを減らす。

### 実施内容

1. `docs/privacy-artifacts.md` を新規作成
2. 明記する内容
   - 保存する情報
     - scene snapshot
     - surface registry
     - watchdog/session artifacts
     - selection owner / payload id / serial などの metadata
   - 保存しない情報
     - clipboard 本文そのものは保存対象にしない方針
     - 入力本文や画面内容の永続保存を目的としないこと
   - 保存先
     - `XDG_RUNTIME_DIR/waybroker` 優先
     - fallback は temp dir
   - 保持期間
     - runtime artifact であり恒久保存を前提としないこと
   - 削除方法
     - runtime dir の掃除方法
3. `README.md` からリンク

### 完了条件
- 利用者が「何が保存され、何が保存されないか」を repo 内 docs だけで理解できる

---

## タスク 4: runtime dir の安全方針を固定

### 目的
artifact が他ユーザ可視になる事故や、temp dir fallback 時の扱いを明確化する。

### 実施内容

1. `ensure_runtime_dir()` 周辺の運用方針を docs 化
2. `docs/runtime-security.md` を新規作成し、以下を明記
   - 推奨は `XDG_RUNTIME_DIR`
   - shared temp 環境では専用 runtime dir を明示指定すること
   - artifact は機微な UI metadata を含み得ること
3. 可能なら実装も最小修正
   - runtime dir 作成後に権限を `0700` 相当に寄せる
   - 不可能なら docs で「OS/環境依存のため利用者が権限制御する」旨を明記

### 完了条件
- runtime artifact の安全運用方針が docs または実装で固定される

---

## タスク 5: README の対外表現を OSS 配布向けに整える

### 目的
第三者が読んだときに、過剰保証・誤認・商標誤読を避ける。

### 実施内容

1. `README.md` に短い「Legal / Distribution Notes」節を追加
2. 内容は以下に限定
   - 本repoは `MIT OR Apache-2.0`
   - third-party dependency licenses は `THIRD_PARTY_NOTICES.md` を参照
   - runtime artifacts may contain operational metadata
   - 本repoは `KDE Plasma`, `GNOME`, `Wayland`, `X11` 等との互換・研究・分離設計を扱うが、各プロジェクトの endorsement を意味しない
3. 不要な免責の羅列はしない

### 完了条件
- README が公開配布向けの最低限の法務表現を持つ

---

## タスク 6: 成果を status doc に固定

### 目的
法務整備が終わったことを commit 単位で追えるようにする。

### 実施内容

1. `docs/status/LEGAL_PUBLICATION_BASELINE_2026-04-04.md` を新規作成
2. 以下を記載
   - 対象 commit SHA
   - 追加した doc / config / CI step
   - 実行コマンド
   - PASS / FAIL
   - 未解決項目があれば簡潔に列挙

### 完了条件
- 法務整備の基準線が repo 内で固定される

---

## 今回の方針

- 実装そのものを後退させない
- 機能削除ではなく、公開・配布に必要な追跡可能性を足す
- 「侵害しない意思」を、docs / CI / notices として証跡化する

---

## 最終完了条件

- [ ] 依存ライセンス監査が CI に入っている
- [ ] `THIRD_PARTY_NOTICES.md` がある
- [ ] 必要なら `NOTICE` がある
- [ ] `docs/privacy-artifacts.md` がある
- [ ] `docs/runtime-security.md` がある
- [ ] README に公開配布向け法務表現がある
- [ ] `docs/status/LEGAL_PUBLICATION_BASELINE_2026-04-04.md` がある

---

## 最終報告フォーマット

### 1. 変更概要
- 何を追加/修正したか

### 2. 追加ファイル
- docs / config / workflow の一覧

### 3. 実行結果
- 実行コマンド
- PASS / FAIL

### 4. 未解決項目
- あれば簡潔に
