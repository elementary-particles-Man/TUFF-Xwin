# Watchdog Auto-Recovery Wiring Status (2026-03-27) [Merged to main]

## 概要
Resume シーケンス中の故障（`restart-request` 状態）を Watchdog へ通知し、Watchdog がリカバリ計画を確定させるまでの配線を完了した。
本成果は `main` ブランチに統合済み。

## 実装内容
- **sessiond**:
  - `compd-trouble` シナリオ等で `restart-request` に到達した際、Watchdog へ `Restart` コマンドを送信。
  - レジュメトレース (`resume-trace-*.json`) に Watchdog へのリクエスト結果をステップとして記録。
- **watchdog**:
  - `Restart` コマンドの受信処理を実装。現在は `Compd` および `Lockd` ロールをサポート。
  - 要求受理時に `watchdog-recovery-<role>.json` アーティファクトを生成。
  - 受理ログの構造化（`service=watchdog op=recovery_request event=accepted ...`）。

## 検証済みフロー
1. `compd` が Resume 中に失敗をシミュレート（`--fail-resume`）。
2. `sessiond` が `restart-request` 判定を行い、Watchdog へ `Restart { role: Compd }` を送信。
3. Watchdog が要求を受理し、`status: pending` で `watchdog-recovery-compd.json` を出力。
4. `manage-active` モードの `sessiond` が要求を検知し、実際に対象コンポーネントを再起動。
5. `watchdog-action-execution-compd.json` (result: succeeded) を出力して終了。

## 検証スクリプト
- `scripts/run-watchdog-auto-recovery.sh`: 配線の検証。
- `scripts/run-role-scoped-recovery-execution.sh`: 実実行の検証。

## 生成アーティファクト例 (`watchdog-recovery-compd.json`)
```json
{
  "role": "compd",
  "reason": "resume failure (restart-request)",
  "requested_by": "sessiond",
  "unix_timestamp": 1774571047,
  "action": "restart-request-accepted"
}
```
