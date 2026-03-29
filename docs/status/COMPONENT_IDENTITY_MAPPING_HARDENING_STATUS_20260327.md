# Component Identity Mapping Hardening Status (2026-03-27) [Merged to main]

## 概要
ServiceRole から実コンポーネントへの解決を、曖昧な role 推測から profile で定義された明示的なバインディング（`service_component_bindings`）へと移行した。これにより、リカバリ対象の特定が決定論的になり、将来的なコンポーネント構成の複雑化に対しても堅牢な基盤が整った。
本成果は `main` ブランチに統合済み。

## 実装内容
- **Schema 拡張**:
  - `DesktopProfile` / `SessionLaunchState` に `service_component_bindings` フィールドを追加。
  - ServiceRole（例: `compd`）と component_id（例: `demo-wm`）の 1対1 束縛を明示。
- **解決ロジックの刷新**:
  - `sessiond` スーパーバイザーがリカバリ時にバインディング情報を優先参照。
  - バインディング不在時の legacy fallback パスを明示化し、警告ログを出力。
- **観測性の向上**:
  - `watchdog-action-execution-<role>.json` に `resolution_source` と `bound_component_id` を追加。

## 移行済みプロファイル
- `demo-x11`: compd -> demo-wm, x11bridge -> demo-x11bridge
- `demo-x11-crashy`: compd -> crashy-wm
- `demo-x11-degraded`: compd -> demo-fallback-wm, x11bridge -> demo-x11bridge-fallback

## 検証結果
- `scripts/run-component-identity-mapping-smoke.sh` により、明示的バインディングに基づいたリカバリ実行（`resolution_source: explicit`）が完走することを確認済み。

## 今後の課題
- Lockd を含めた全コンポーネントの明示的バインディング化の完了。
- 同一 role を持つ複数コンポーネントの同時リカバリ要求への対応。
