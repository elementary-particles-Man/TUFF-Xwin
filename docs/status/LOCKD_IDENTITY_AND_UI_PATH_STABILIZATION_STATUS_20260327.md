# Lockd Identity and UI Path Stabilization Status (2026-03-27)

## 概要
Lockd を explicit binding と専用 component identity (`LockScreen` role) で安定化させました。resume 時の lock/auth/blank-only 経路において、どの UI component が使用されたか、また binding 不在時にどのような振る舞いをするかが決定論的かつ観測可能になりました。

## 実装内容
- **Component Identity**: `DesktopComponentRole` に `LockScreen` (serde name: `lockscreen`) を追加しました。
- **Explicit Binding**: Demo プロファイル (`demo-x11`, `demo-x11-crashy`, `demo-x11-degraded`) に専用の lock UI component (`demo-lockui` / `demo-lockui-fallback`) を追加し、`ServiceRole::Lockd` への明示的な binding を定義しました。
- **Path Observability**: `LockPathArtifact` (`lock-ui-path-<scenario>.json`) を出力するようにし、Lockd 関連の step の結果や使用された binding 情報を追跡可能にしました。
- **Policy Enforcement**: `Lockd` の binding が存在しない場合は、legacy fallback を行わずに決定論的に `blank-only` へフォールバックするポリシーを `sessiond` に実装しました。

## 検証スクリプト
- `scripts/run-lockd-identity-and-ui-path-smoke.sh`: Normal パス、Lockd Trouble パス、および binding が見つからない場合のフォールバック挙動を一括で検証します。

## 検証済みシナリオ
- **lockd-normal-with-binding**: `demo-lockui` を通じた正常な lock/auth 遷移。
- **lockd-trouble-with-binding**: `demo-lockui` をバインドしているが `lockd` が異常（fail-resume）な場合、`blank-only` になることを確認。
- **lockd-without-binding**: Profile に `lockd` の binding がない場合、素早く `blank-only` にフォールバックすることを確認。

## 今後の課題
- 実際の PAM 連携やより本格的な Lock UI 実装。
- 必要であれば、Lockd に対する Role-Scoped Recovery Execution の追加対応（現在は optional として留めている）。
