# TUFF-Xwin

`TUFF-Xwin` はリポジトリ名であり、表示系アーキテクチャの正式名は `Waybroker` です。狙いは `Debian Linux kernel` を置き換えることではなく、`KDE Plasma` や `GNOME` の display stack を `displayd / waylandd / compd / lockd / sessiond` に分割し、単一障害点を細くすることです。

## Repository Layout

```text
TUFF-Xwin/
├── Cargo.toml
├── crates/
│   ├── waybroker-common/
│   ├── displayd/
│   ├── waylandd/
│   ├── compd/
│   ├── lockd/
│   ├── sessiond/
│   └── watchdog/
├── docs/
├── examples/
├── scripts/
└── .github/workflows/
```

## Workspace Members

- `waybroker-common`: 共通型と service metadata
- `displayd`: `DRM/KMS`、`input`、`seat` の broker
- `waylandd`: Wayland 接続口と object lifetime 管理
- `compd`: scene、focus、composition policy
- `lockd`: lockscreen と認証 UI
- `sessiond`: lid、idle、power、polkit policy
- `watchdog`: display stack の監視と再起動制御

## Documentation

- [docs/README.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/README.md)
- [docs/design-memo.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/design-memo.md)
- [docs/repo-layout.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/repo-layout.md)

## Quick Start

```bash
cargo check
./scripts/dev-check.sh
```

現時点では設計骨格と Rust workspace の初期化までです。実装はまだ入っていません。

## Local Build Note

repository は `CIFS` 共有上にあるため、`cargo target-dir` は `.cargo/config.toml` で `/home/flux/.cache/tuff-xwin-target` に逃がしています。source tree は共有上に置いたまま、build artifact だけローカル実行可能領域を使う想定です。
