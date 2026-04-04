# TUFF-Xwin

`TUFF-Xwin` はリポジトリ名であり、表示系アーキテクチャの正式名は `Waybroker` です。狙いは `Debian Linux kernel` を置き換えることではなく、`KDE Plasma` や `GNOME` の display stack を `displayd / waylandd / compd / lockd / sessiond` に分割し、単一障害点を細くすることです。

## Current Component Status

- **compd**: (P1 - Minimal runtime) Composition service. Manages scene graph, focus, and policies. Can commit mock scenes to `displayd`, rebuild from `displayd` plus `waylandd`, hand off selection ownership, and re-commit rebuilt scene during supervisor restart.
- **displayd**: (P0 - Baseline) Hardware broker for DRM/KMS and libinput. Stub but functional IPC, with persisted last-scene snapshot.
- **waylandd**: (P1 - Minimal runtime) Wayland protocol endpoint and surface broker. Can serve surface-registry snapshots for `compd` rebuild and accept post-recovery focus/selection handoff.
- **sessiond**: (P0 - Baseline) Session and desktop profile policy manager.
- **watchdog**: (P0 - Baseline) Recovery orchestrator and crash-loop supervisor.
- **lockd**: (Stub) Lockscreen and authentication UI service.
- **x11bridge**: (Demo) Rootless X11 compatibility island (LeyerX11).

## Repository Layout

```text
TUFF-Xwin/
├── Cargo.toml
├── LeyerX11/
├── profiles/
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
- `sessiond`: lid、idle、power、polkit policy、desktop profile manager
- `watchdog`: display stack の監視と再起動制御
- `LeyerX11/layerx11-common`: rootless `X11` scene の共通型
- `LeyerX11/x11bridge`: optional な `X11` 互換レイヤ実験
- `profiles/`: 選択可能な GUI profile manifest

## Documentation

- [docs/README.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/README.md)
- [docs/design-memo.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/design-memo.md)
- [docs/repo-layout.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/repo-layout.md)
- [docs/api-boundary.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/api-boundary.md)
- [docs/sequence-resume.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/sequence-resume.md)
- [docs/ipc-format.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/ipc-format.md)
- [docs/crash-loop-policy.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/crash-loop-policy.md)
- [docs/desktop-profiles.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/desktop-profiles.md)
- [LeyerX11/README.md](/media/flux/THPDOC/Develop/TUFF-Xwin/LeyerX11/README.md)
- [profiles/README.md](/media/flux/THPDOC/Develop/TUFF-Xwin/profiles/README.md)
- [CONTRIBUTING.md](/media/flux/THPDOC/Develop/TUFF-Xwin/CONTRIBUTING.md)

## Quick Start

```bash
cargo check
./scripts/dev-check.sh
./scripts/run-integration-smoke.sh
./scripts/run-resume-scenarios.sh
./scripts/run-watchdog-auto-recovery.sh
./scripts/run-role-scoped-recovery-execution.sh
./scripts/run-component-identity-mapping-smoke.sh
./scripts/run-lockd-identity-and-ui-path-smoke.sh
./scripts/run-lockd-recovery-execution-optionalization.sh
./scripts/run-stack.sh
./scripts/run-scene-recovery-demo.sh
./scripts/run-compd-broker-recovery.sh
./scripts/run-profile-demo.sh
./LeyerX11/scripts/run-rootless-demo.sh
```

現時点では `displayd` / `waylandd` の最小 IPC、`displayd` の last-scene snapshot、`waylandd` の surface-registry + selection snapshot、`sessiond/watchdog` 経由の `compd` broker restart + startup rebuild + selection handoff、ならびに `LeyerX11` の rootless `X11` commit デモまで入っています。本物の `DRM` / `Wayland` / `X11` 実装はこれからです。

## Local Build Note

repository は `CIFS` 共有上にあるため、`cargo target-dir` は `.cargo/config.toml` で `/home/flux/.cache/tuff-xwin-target` に逃がしています。source tree は共有上に置いたまま、build artifact だけローカル実行可能領域を使う想定です。

## License

この repository は `MIT OR Apache-2.0` の dual license です。

- [LICENSE-MIT](/media/flux/THPDOC/Develop/TUFF-Xwin/LICENSE-MIT)
- [LICENSE-APACHE](/media/flux/THPDOC/Develop/TUFF-Xwin/LICENSE-APACHE)
