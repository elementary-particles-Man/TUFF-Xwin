# CLI Agent Task — Final Pass Criteria for TUFF-Xwin

更新日: `2026-04-04`
対象 repository: `elementary-particles-Man/TUFF-Xwin`

## 目的

外部レビューの結果、現状の TUFF-Xwin は **概ね完成** だが、以下 2 点が残っているため **完全合格** ではありません。

- `watchdog` の recovery restart IPC が **role-only** で、multi-session 環境で対象 session を厳密に特定できない
- `session_instance_id` が artifact path に **ほぼ生値のまま** 入っており、CLI 引数由来の unsafe 文字列に対する防御が弱い

この 2 点を **最小差分で是正** し、必要なテストとドキュメント更新まで完了してください。

---

## 作業範囲

### A. Recovery request を session-aware にする

#### 要件

`WatchdogCommand::Restart` または同等の recovery request 経路に、`session_instance_id` を明示的に乗せてください。

現状は `watchdog` 側で「latest session に当てる」暫定処理になっています。これを本実装へ置き換えてください。

#### 必須条件

1. recovery request 発行側から `session_instance_id` を指定できること
2. `watchdog` がその `session_instance_id` を用いて **対象 session のみ** を recovery 対象にすること
3. multi-session 並列時に、別 session の `compd` / `lockd` を誤って触らないこと
4. ログと artifact に `session_instance_id` が明示されること
5. 旧データ互換が必要なら、legacy decode は残してよいが、**新経路では role-only fallback に依存しないこと**

#### 実装ヒント

- `WatchdogCommand::Restart` に `session_instance_id: String` を追加するのが第一候補
- request 発行箇所、IPC enum、server 側 dispatch、artifact 出力、ログ出力を一貫して更新
- `watchdog` の `latest_session_instance_id()` 依存は原則撤去。どうしても残すなら **legacy fallback 限定** に閉じ込めること

---

### B. `session_instance_id` を path-safe にする

#### 要件

`session_artifact_path(session_instance_id, artifact_name)` へ渡る `session_instance_id` に対し、**path-safe な正規化/検証** を導入してください。

#### 必須条件

1. `/`, `\\`, `..`, NUL 相当、制御文字、極端な長さなどで path が壊れないこと
2. artifact path 生成が常に runtime dir 配下へ閉じること
3. CLI 引数 `--session-instance-id` から unsafe 値を与えても、
   - reject する
   - あるいは deterministic に sanitize する
   のどちらかで安全に処理すること
4. 挙動がテストで固定されていること

#### 推奨方針

- allowlist 方式推奨: `[A-Za-z0-9._-]` のみ許可
- それ以外は `_` へ置換、あるいは入力全体を reject
- 文字数上限も設けること（例: 64 〜 128）

---

## テスト要件

以下を **追加または更新** してください。

### 1. session-aware recovery test

少なくとも以下をカバーしてください。

- session A / session B の launch state を並列で持つ
- session A 向け recovery request を送る
- session B は無変更であることを確認
- recovery artifact / log / response が session A を指していることを確認

### 2. session_instance_id sanitization test

少なくとも以下をカバーしてください。

- 正常系: `default-single-session`, `abc-123`, `sess.demo_01`
- 異常系: `../evil`, `a/b`, `a\\b`, 空文字, 過剰長, 制御文字を含む値
- 生成 path が runtime dir 外へ出ないこと

### 3. 既存 smoke の回帰確認

少なくとも以下を回してください。

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
./scripts/dev-check.sh
```

既存スクリプトに multi-session / watchdog recovery 系があるなら、そこも必要最小限で回してください。

---

## ドキュメント更新

修正後、以下を更新してください。

1. `README.md`
   - multi-session recovery の扱いが session-aware になったことを反映
2. `HANDOFF.md`
   - 「未完了事項」からこの 2 点を除外
   - 追加したテスト/注意点があれば簡潔に記載

必要なら新規 status doc を追加して構いません。

---

## 完了条件

以下をすべて満たしたら完了です。

- [ ] recovery request が session-aware
- [ ] role-only fallback 依存が本経路から外れている
- [ ] `session_instance_id` が path-safe
- [ ] 上記 2 点に対するテストが追加済み
- [ ] workspace check/test が通過
- [ ] README / HANDOFF が更新済み

---

## 最終報告フォーマット

作業完了時は、以下の形式で簡潔に報告してください。

### 1. 変更概要
- 何をどう直したか

### 2. 主要ファイル
- 変更ファイル一覧

### 3. テスト結果
- 実行コマンド
- PASS / FAIL

### 4. 残課題
- ある場合のみ簡潔に

---

## 重要

- スコープを広げすぎないでください
- 今回は **完全合格を阻んでいる 2 点の是正** が主目的です
- Vulkan 実シェーダー導入のような別フェーズ作業へは広げないでください
