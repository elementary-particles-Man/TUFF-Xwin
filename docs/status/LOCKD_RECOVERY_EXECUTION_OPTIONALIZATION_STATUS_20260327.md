# Lockd Recovery Execution Optionalization Status (2026-03-27) [Merged to main]

## 概要
Lockd の explicit binding と UI path stabilization を前提に、Lockd のリカバリ実行（再起動）を「既定では無効」「profile での明示 opt-in 時のみ有効」「final_state=blank-only は維持」という安全モデルで追加しました。これにより、Lockd trouble 時の画面状態は安全側に倒しつつ、必要な構成だけがバックグラウンドで lockd 再起動を試みられるようになりました。

## 実装内容
- **Policy Schema**: `DesktopProfile` に `service_recovery_execution_policies` を追加し、`RecoveryExecutionMode` (`disabled` | `supervisor-restart`) を定義しました。
- **Opt-in Execution**: `sessiond` が Lockd の restart-request を送るか、および実際にリカバリを実行するかどうかを、このポリシーに基づいて判定するようにしました。
- **Default Safety**: Policy が未設定の場合、Compd は既存の挙動維持のため `supervisor-restart` として扱われますが、Lockd は `disabled` として扱われます。
- **Observability**: `lock-ui-path-<scenario>.json` に `execution_policy`, `watchdog_request_outcome`, `execution_result` を追加し、ポリシーの評価結果と実行経路を観測可能にしました。

## 新規プロファイル
- `demo-x11-lockd-recovery-optin.json`: Lockd のリカバリ実行を opt-in で有効化したデモ検証用プロファイルを追加しました。

## 検証スクリプト
- `scripts/run-lockd-recovery-execution-optionalization.sh`:
  - **default-disabled**: 既定では execution が行われず、`watchdog_request=skipped`, `execution_result=none` となることを確認。
  - **optin-enabled**: Opt-in プロファイルでは、`watchdog_request=accepted`, `execution_result=succeeded` まで進むことを確認。
  - **optin-missing-binding**: Policy は enabled だが Binding が missing の場合、`execution_result=no-executor` となることを確認。

## 今後の課題
- 同一 role を持つ複数コンポーネントの同時リカバリ要求への対応や、ServiceRole -> Component マッピングの更なる厳密化（P8）。