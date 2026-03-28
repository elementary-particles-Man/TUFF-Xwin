# Post-Merge Baseline Status Report (2026-03-27)

## 概要
`feat/compd-minimal-runtime` ブランチを `main` へ Fast-Forward マージし、開発基準を `main` 一本に統合した。
マージ後の `main` ブランチにおいて、全てのワークスペースチェック（fmt, check, test）および主要なデモスクリプトが正常に動作することを確認した。

## マージ結果
- **Source Branch**: `feat/compd-minimal-runtime`
- **Target Branch**: `main`
- **Mode**: Fast-Forward
- **Status**: Success
- **Branch Deletion**: Local and Remote branches deleted.

## 検証結果 (main branch)

| Item | Result | Evidence |
|:---|:---|:---|
| cargo fmt --all --check | PASS | No diff found |
| cargo check --workspace | PASS | No errors |
| cargo test --workspace | PASS | All 30 tests passed |
| ./scripts/run-stack.sh | PASS | displayd/waylandd/compd interaction verified |
| ./scripts/run-profile-demo.sh | PASS | sessiond profile selection and component launch verified |
| ./scripts/run-watchdog-resync-demo.sh | PASS | sessiond <-> watchdog resync and degraded transition verified |

## 変更のハイライト
- **compd**: 最小ランタイムが実装され、`displayd` へのモックシーンコミットが可能になった。
- **Resume/Degraded Hardening**: Resume 時の故障分類とシナリオ検証環境が導入された。`sessiond` によるオーケストレーションとトレース生成が `main` に一本化された。
- **Watchdog Auto-Recovery Wiring**: `restart-request` 状態が Watchdog へ通知され、リカバリ計画がアーティファクトとして記録されるフローが `main` に統合された。
- **Role-Scoped Recovery Execution**: Watchdog が受理したリカバリ要求に基づき、`manage-active` なスーパーバイザーが対象コンポーネントを実際に再起動するフローが `main` に統合された。
- **IPC**: `SessionLaunchState` および `SessionLaunchDelta` に `unix_timestamp` が追加され、watchdog 連携の堅牢性が向上した。
- **Tests**: 構造体の変更に追従し、全てのユニットテストが最新化された。

## 次のステップ
- `P5-COMPONENT-IDENTITY-AND-ROLE-MAPPING-HARDENING`: ServiceRole から実コンポーネントへの解決を、曖昧性を排除したより堅牢なマッピング方式へ刷新する。
