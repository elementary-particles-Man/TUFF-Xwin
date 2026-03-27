# Resume Scenario Status Report (2026-03-27) [Merged to main]

## 概要
Resume 時の異常系ハンドリングを強化し、シナリオベースの検証環境を構築した。
本成果は `main` ブランチに統合済み。

## 実装済みシナリオ

| Scenario | Trigger | Final State | Description |
|:---|:---|:---|:---|
| `normal` | None | `normal` | 全てのサービスが正常に応答する基本パス。 |
| `displayd-trouble` | `displayd --fail-resume` | `hold` | `displayd` が Resume 開始を拒否した場合。出力を保護するため現状維持。 |
| `compd-trouble` | `compd --fail-resume` | `restart-request` | `compd` が resume-hint に失敗を返した場合。Compositor の再起動を要求する。 |
| `lockd-trouble` | `lockd --fail-resume` | `blank-only` | `lockd` が状態遷移または認証プロンプトに失敗した場合。安全のため画面を隠す。 |

## 生成アーティファクト
`WAYBROKER_RUNTIME_DIR` に `resume-trace-<scenario>.json` が書き出される。
これにはシナリオ名、タイムスタンプ、各ステップの結果、および最終状態が含まれる。

## 検証結果
`scripts/run-resume-scenarios.sh` により、上記 4 シナリオが全て期待通りの `final_state` で終了することを確認済み。

## 今後の課題
- `watchdog` との密連携による `restart-request` 時の自動復旧の実装。
- 実際のハードウェアエラーを模擬したより詳細な故障分類。
