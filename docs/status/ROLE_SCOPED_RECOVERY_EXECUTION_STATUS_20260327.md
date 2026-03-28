# Role-Scoped Recovery Execution Status (2026-03-27)

## 概要
Watchdog が受理したリカバリ要求（restart-request）に基づき、`manage-active` な `sessiond` スーパーバイザーが対象コンポーネントを実際に再起動するフローを実装・検証した。これにより、「監視 -> 受理 -> 実行」のリカバリライフサイクルが完走するようになった。

## 実装内容
- **sessiond (Recovery Executor)**:
  - スーパーバイザーループ内で `watchdog-recovery-*.json` を監視。
  - `ServiceRole` から実コンポーネントへの解決（例: `Compd` -> `window-manager` ロールを持つコンポーネント）。
  - `RuntimeComponent` を用いた安全な停止（kill/wait）と再起動（spawn）。
  - 実行結果の永続化（`watchdog-action-execution-<role>.json`）。
- **watchdog (Artifact Schema)**:
  - スーパーバイザーとの受け渡しを安定させるため `status` フィールド（初期値 `pending`）を追加。

## 検証済みフロー
1. `compd` の Resume 失敗をトリガーに `restart-request` が発生。
2. Watchdog が要求を受理し、`status: pending` のアーティファクトを作成。
3. `manage-active` モードの `sessiond` がアーティファクトを検出し、`demo-wm` を再起動。
4. `watchdog-action-execution-compd.json` に `result: succeeded` が記録される。

## 検証スクリプト
- `scripts/run-role-scoped-recovery-execution.sh`: 故障注入から再起動完了までの End-to-End 検証。

## 生成アーティファクト例 (`watchdog-action-execution-compd.json`)
```json
{
  "role": "compd",
  "action": "restart",
  "requested_at": 1774705628,
  "executed_at": 1774705628,
  "result": "succeeded",
  "component_id": "demo-wm",
  "previous_pid": 860152,
  "new_pid": 860553,
  "reason": "component restarted successfully"
}
```
