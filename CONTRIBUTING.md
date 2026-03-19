# Contributing

## Scope

`TUFF-Xwin` は `Waybroker` アーキテクチャの設計と実装骨格を育てる repository です。現時点では「全部盛りの desktop replacement」を目指すより、責務分離と故障半径の縮小に集中します。

## Before Opening A Pull Request

1. 変更が `displayd / waylandd / compd / lockd / sessiond / watchdog` のどこに属するかを明確にしてください。
2. 変更が fault isolation を改善するのか、単に機能を増やすだけなのかを説明してください。
3. `docs/` 側の設計差分が必要なら、コードと同じ PR で更新してください。
4. `./scripts/dev-check.sh` を通してください。

## Design Rules

- `displayd` に policy を積まない
- `compd` に hardware ownership を持たせない
- `lockd` を display stack 本体と運命共同体にしない
- `sessiond` に UX 補助機能を寄せても、表示中枢には戻さない
- 障害時の復旧経路を削る変更は避ける

## Commit And PR Guidance

- commit message は短く、変更の軸を明示してください
- PR では「何が変わったか」より「障害半径がどう変わるか」を先に書いてください
- 仕様変更は `docs/` を先に更新するか、少なくとも同時に更新してください

## Licensing

特に明記しない限り、この repository への contribution は `MIT OR Apache-2.0` で提供される前提です。
