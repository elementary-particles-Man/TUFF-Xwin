# Multi-Component Role Collision Handling Status (2026-03-27)

## 概要
同一 ServiceRole に対する複数バインディングや、コンポーネント ID の不在、ロールの不一致などを決定論的に検出し、曖昧な状態でのリカバリ実行を安全にブロックする機能を実装しました。これにより、プロファイル構成の誤りによる予期せぬ再起動を防ぎ、システムの堅牢性が向上しました。

## 実装内容
- **Binding Validation Layer**: プロファイル読込時およびリカバリ実行直前に、バインディングの重複・不足・不整合をチェックするバリデーション層を `sessiond` に導入しました。
- **Deterministic Severity Policy**: 衝突検出時の挙動を固定しました。
  - **Collision (重複)**: 実行をブロックし `result: config-error` (or `collision`) を記録。
  - **Missing Target**: 対象コンポーネント不在時は実行をブロックし `result: config-error` (or `missing-target`) を記録。
  - **Role Mismatch**: ロール不一致時は実行をブロックし `result: config-error` (or `role-mismatch`) を記録。
- **Resolution Artifacts**:
  - `binding-collision-report.json`: プロファイル全体のバリデーション結果を要約。
  - `binding-resolution-<service>.json`: サービスごとの解決過程（候補 ID、選択結果、理由）を詳細に記録。

## 検証スクリプト
- `scripts/run-binding-collision-handling-smoke.sh`:
  - `compd-binding-collision`: 複数バインディング時にリカバリが阻止されることを確認。
  - `lockd-binding-collision`: 同様に Lockd での衝突を検証。
  - `lockd-missing-target`: バインディング先が存在しない場合の決定論的な失敗を検証。

## 生成アーティファクト例 (`binding-resolution-compd.json`)
```json
{
  "service": "compd",
  "resolution_source": "explicit",
  "candidate_component_ids": ["demo-wm-1", "demo-wm-2"],
  "selected_component_id": null,
  "result": "collision",
  "reason": "multiple bindings found for service",
  "unix_timestamp": 1774914301
}
```

## 今後の課題
- 複雑なプロファイルにおけるロールの曖昧性（複数コンポーネントが同一ロールを持つがバインディングがないケース等）への対応強化。
