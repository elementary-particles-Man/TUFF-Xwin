# TUFF-Xwin / KDE Plasma 受け皿化ロードマップ

## 目的

TUFF-Xwin を、KDE Plasma が接続できる実用的な Wayland display stack に段階的に発展させる。

最終目標は、KWin を表示スタックの単一障害点にせず、次の責務を分離した構成を実現することである。

```text
KDE Plasma / Qt / GTK / Xwayland applications
                         |
                  TUFF-Xwin Wayland endpoint
                         |
               waylandd <-> compd
                         |
                      displayd
                         |
                    DRM/KMS/input
```

この文書でいう「Plasma の受け皿」とは、単に UNIX socket を開くことではない。Wayland protocol を処理し、surface/buffer を合成し、入力を返し、Qt/GTK/Xwayland/Plasma Shell を実際に動かせることを意味する。

## 現状認識

- `displayd` / `waylandd` / `compd` は broker と protocol model の基盤である。
- `waylandd --bind-wayland-display` は現在、接続を受け付ける診断用 listener に留まる。
- `wayland-wire` には protocol と object model があるが、本番 Wayland client endpoint との接続は未完成である。
- `compd` は scene commit と recovery の基盤を持つが、完成した compositor/window manager ではない。
- `host-wayland` profile の GUI component は明示指定されない場合 `sleep infinity` になる。
- `takeover` は現時点では実 compositor の存在を保証しないため、通常は実行してはならない。
- 再起動後に KWin が復帰し、transient な TUFF-Xwin unit が消えることは想定された挙動である。
- GPU/Vulkan は表示スタック完成後に最適化する。CPU fallback を常に動作基準にする。

## 実装原則

1. KWin を止めずに検証できる状態を維持する。
2. headless または nested compositor を先に完成させ、実 DRM/KMS は後段にする。
3. 各フェーズに機械的な exit criteria を置く。
4. fake backend、diagnostic listener、本番 endpoint を明確に分離する。
5. protocol の未実装機能は黙って成功させず、明示的な protocol error または機能未対応として扱う。
6. `Result` と recovery path を使い、panic や無限再試行を通常経路にしない。
7. 変更は小さく行い、`fmt`、`check`、unit test、integration smoke test の順で検証する。
8. display、input、lock、session、policy の権限境界を維持する。
9. 既存のユーザー変更を上書きしない。
10. `.cargo/config.toml` を削除しない。CIFS share のため target directory redirect が必要である。

## Phase 0: 現状固定と安全境界

### 目的

未完成の Xwin によって KWin の画面を失わないようにし、今後の実装を KWin 併用状態で検証できるようにする。

### 作業

- `takeover` に実 compositor readiness check を追加する。
- readiness check が確認する条件を定義する。
  - 実 Wayland endpoint が存在する。
  - protocol client の接続試験が成功する。
  - output が一つ以上存在する。
  - 単一 surface の commit と presentation が成功する。
- `--bind-wayland-display` を diagnostic listener として明示的に命名・文書化する。
- headless/nested 用の専用 runtime directory と socket を用意する。
- CPU backend を基準にした再現可能な smoke test を追加する。

### 完了条件

- KWin を停止せず、TUFF-Xwin の broker 群だけを起動できる。
- readiness check 未通過時の `takeover` が fail closed になる。
- `./scripts/dev-check.sh` が成功する。

## Phase 1: 実 Wayland endpoint

### 目的

Wayland client が実際に protocol 通信できる endpoint を `waylandd` に実装する。

### 最小 protocol 範囲

- `wl_display`
- `wl_registry`
- `wl_compositor`
- `wl_shm`
- `wl_shm_pool`
- `wl_buffer`
- `wl_surface`
- `xdg_wm_base`
- `xdg_surface`
- `xdg_toplevel`

### 作業

- UNIX socket listener と client event loop を実装する。
- Wayland wire message の decode/encode、FD passing、object id 管理を実装する。
- `wl_display` の hello/sync/get_registry/error/delete_id を実装する。
- surface の attach、damage、commit、release を処理する。
- configure/ack_configure の順序を検証する。
- client disconnect、protocol error、resource cleanup を実装する。
- 既存 `wayland-wire` の model を再利用し、重複した protocol 定義を増やさない。

### 完了条件

- 小さな Wayland client が接続できる。
- client が surface を作成し、buffer commit を送れる。
- 不正な message が protocol error になり、`waylandd` 全体は生存する。
- 複数 client の接続と切断を繰り返しても object state が残らない。

## Phase 2: 最小 compositor / scene pipeline

### 目的

`waylandd -> compd -> displayd` の経路で、単一 surface を実際に表示できるようにする。

### 作業

- surface tree と toplevel lifecycle を `compd` に実装する。
- surface geometry、visibility、stacking order、focus を管理する。
- damage tracking と frame callback の最小実装を追加する。
- surface buffer を scene snapshot に変換する。
- `displayd` の `CommitScene` に buffer metadata と damage を渡す。
- `displayd` から presentation feedback を返す。
- `compd` crash 後に displayd snapshot から最低限の scene を復元する。

### 完了条件

- 単一 window が表示される。
- 2つ以上の window を配置できる。
- client の frame callback が適切に返る。
- `compd` を停止・再起動しても kernel と displayd が生存する。
- scene snapshot の復元テストが成功する。

## Phase 3: output backend

### 目的

仮想 output から nested compositor、最後に実 DRM/KMS へ段階的に進む。

### 順序

1. headless output
2. nested Wayland output
3. software/CPU composition
4. 実 DRM/KMS output
5. mode change と hotplug

### 作業

- output inventory と mode の実状態を定義する。
- CPU composition を reference implementation にする。
- frame buffer の lifetime と ownership を定義する。
- DRM/KMS access を `displayd` に限定する。
- `compd` が DRM/KMS を直接触らないことをテストする。
- Vulkan backend は CPU 結果との比較テスト後に有効化する。

### 完了条件

- headless output で pixel artifact を検証できる。
- nested output で実際の frame が表示される。
- CPU backend で deterministic な capture ができる。
- DRM/KMS backend の失敗が displayd の再起動で閉じる。

## Phase 4: input / clipboard / Xwayland

### 目的

GUI application を実用的に動かすための基本互換性を実装する。

### 作業

- pointer、keyboard、focus、relative pointer を実装する。
- clipboard、primary selection、DnD を `waylandd` 経由で実装する。
- text-input/IME の最小経路を実装する。
- rootless Xwayland を lifecycle 管理する。
- X11 window を Wayland scene に変換する。
- 最小 EWMH/ICCCM と cursor 処理を追加する。
- Xwayland crash を Wayland native client から分離する。

### 完了条件

- terminal、Qt app、GTK app が表示される。
- キーボード、pointer、copy/paste が動く。
- X11 app が Xwayland 経由で表示される。
- Xwayland を kill しても Wayland native client と broker が生存する。

## Phase 5: Plasma Shell 接続

### 目的

KWin を停止した状態で、Plasma Shell を Xwin の Wayland endpoint に接続する。

### 最初に対象にする機能

- `plasmashell`
- panel
- wallpaper
- notification
- application launcher
- 基本の Qt/GTK application
- `xdg-desktop-portal-kde` の基本経路

### 後回しにする機能

- KWin 独自 effects
- 完全な tiling
- 高度な decoration
- multi-monitor の全機能
- screen management の全機能
- lockscreen と power policy の完全統合

### 作業

- Plasma が要求する Wayland globals と protocol 拡張をログから特定する。
- 必須拡張だけを優先して実装する。
- 未実装拡張を曖昧に advertise しない。
- `plasmashell` を通常 client として起動する profile を追加する。
- panel、desktop、notification の surface を scene に取り込む。
- `powerdevil`、`kglobalaccel`、portal の責務を sessiond/lockd と整理する。

### 完了条件

- KWin を停止しても `plasmashell` が Xwin endpoint に接続する。
- panel と application window が表示される。
- Qt/GTK/Xwayland app を同時に利用できる。
- Plasma Shell の再起動が session 全体を破壊しない。

## Phase 6: recovery と session 統合

### 目的

Xwin の特徴である故障分離と復旧を実際の Plasma session に適用する。

### 作業

- `waylandd`、`compd`、`displayd`、`plasmashell` の health state を分離する。
- `sessiond` の restart policy と watchdog stream を接続する。
- `compd` 再起動時の scene reconcile を実装する。
- lock state、selection、focus の handoff を検証する。
- crash loop 時の degraded profile を実装する。
- `session_instance_id`、generation、sequence の衝突をテストする。

### 完了条件

- `compd` crash 後も kernel、displayd、sessiond が生存する。
- `waylandd` restart 時に不正な client state が漏れない。
- Plasma Shell の crash が broker crash と区別される。
- crash loop 時に安全な degraded mode へ移行する。
- watchdog cache miss と resync が成功する。

## Phase 7: DRM/KMS、seat、login session

### 目的

既存の KWin session 内での nested 実験から、実 display manager/login session へ移行する。

### 作業

- logind、seat、VT の所有を displayd に限定する。
- input device の権限と lifecycle を定義する。
- suspend/resume、lid、hotplug、multi-monitor を実装する。
- systemd user unit と display manager entry を整備する。
- safe mode、rollback、TTY/SSH recovery を用意する。
- `WAYLAND_DISPLAY`、Xwayland、portal の session environment を固定する。

### 完了条件

- login から Xwin session を選択できる。
- displayd failure 時に TTY/SSH から回復できる。
- suspend/resume 後に output と Plasma が復帰する。
- rollback により通常の KDE Plasma/KWin session に戻れる。

## 検証マトリクス

各フェーズで最低限、次を実行する。

```bash
./scripts/dev-check.sh
./scripts/run-stack.sh
./scripts/run-degraded-mode.sh
./scripts/run-watchdog-resync-demo.sh
```

実 compositor 実装後に追加する検証:

- Wayland protocol client smoke test
- 単一 surface / 複数 surface test
- CPU composition pixel comparison
- nested output test
- Xwayland application test
- Plasma Shell startup test
- compositor crash/restart test
- KWin rollback test

## 指示役 GPT への実装指示

1. 一度に一つの phase だけを進めること。
2. phase の開始時に対象ファイル、依存関係、非対象範囲を示すこと。
3. 実装前に既存の `AGENTS.md`、`HANDOFF.md`、`docs/README.md`、`crates/waybroker-common/src/ipc.rs` を確認すること。
4. `.codegraph/` が存在する場合は、grep/find より先に CodeGraph を使って関係する symbol と call path を調べること。
5. 変更は `apply_patch` で小さく行い、既存の未コミット変更を保持すること。
6. KWin、Plasma、GPU driver、display manager を停止・再設定する操作は、実装と検証が十分な場合に限り、明示的な確認を挟むこと。
7. テストが失敗した場合は、失敗原因だけを切り分けて直すこと。
8. 実装完了時は、変更ファイル、検証コマンド、未解決の制約を日本語で報告すること。
9. `takeover` を実 compositor readiness check のない状態で成功扱いにしないこと。
10. 「socket が存在する」「プロセスが起動している」だけでは Plasma 受け皿化の完了と判定しないこと。

## 最初に着手すべき milestone

最初の実装対象は Phase 0 と Phase 1 の境界である。

具体的には、次の小さな milestone から開始する。

> `waylandd` に実 Wayland client endpoint を追加し、最小 client が `wl_registry`、`wl_compositor`、`wl_shm`、`xdg_wm_base`、surface commit まで完了できることを CPU/headless 環境で検証する。

この milestone が通るまでは、KWin の停止、Plasma の takeover、実 DRM/KMS の切り替えを行わない。
