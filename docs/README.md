# Waybroker / TUFF-Xwin

`Waybroker` (internal codename `TUFF-Xwin`) is a conceptual design memo and framework aimed at isolating failures within the display stack (e.g., `KDE Plasma` or `GNOME`) while utilizing the stock `Linux kernel`. The goal is not to write a new kernel, but rather to segment the responsibilities of `display/input/session` so that a crash in the `compositor` does not bring down the entire OS or the active user session.

`Waybroker`（内部コードネーム `TUFF-Xwin`）は、既存の `Linux kernel` をそのまま使いながら、`KDE Plasma` や `GNOME` の表示系を故障分離するための構想およびフレームワークです。目的は新しいカーネルを作ることではなく、`display/input/session` の責務を分割し、`compositor` が落ちても OS 全体や作業セッションを巻き込まない構成を作ることです。

---

## Why is it needed? / なぜ必要か

### English
Current environments like `KWin Wayland` or `GNOME Shell` have tightly coupled responsibilities despite their high integration in normal operation. Display outputs, input routing, window management, lock screen logic, and power management coordinate in a single process, making the compositor a massive single point of failure (SPOF).

Instead of solving this with a "Wayland-specific kernel", `Waybroker` approaches it using a **least-privileged userspace broker** combined with **restartable display services**.

### 日本語
現行の `KWin Wayland` や `GNOME Shell` は、平常時の統合度は高い一方で、障害時には責務が密結合しすぎています。表示、入力、ウィンドウ管理、ロック画面、電源連携が一体化しているため、単一障害点が太すぎます。

`Waybroker` は、この問題を「Wayland 専用カーネル」で解くのではなく、**userspace の最小特権 broker** と **再起動可能な display stack** で解く前提に立ちます。

---

## Design Goals / 設計目標

- Keep the stock `Linux kernel` intact / `Linux kernel` はそのまま使う
- Retain `KDE Plasma` and `GNOME` high-level UX / `KDE Plasma` や `GNOME` の上位 UX は極力維持する
- Do not drop the kernel or session when the compositor crashes / `compositor` が落ちても kernel や session 全体は落とさない
- Separate `lockscreen`, `power management`, and `policy` from the display server / `lockscreen`、`power management`、`policy` を display server 本体から分離する
- Bring `X11`-style failure isolation into the Wayland era / `X11` 的な故障分離を、Wayland 世代の構成に持ち込む

---

## Non-Goals / 非目標

- Modifying the Linux kernel / Linux kernel の書き換え
- Guaranteeing complete compatibility with all desktop environments in early phases / 全デスクトップ環境の完全互換を初期段階から保証すること
- Recovering all kernel deadlocks or driver bugs strictly from userspace / すべての kernel deadlock や driver bug を userspace だけで救うこと
- Running unmodified `KWin` or `Mutter` as-is / 既存 `KWin` や `Mutter` を無改造でそのまま使うこと

---

## Key Overview / 要点

```text
Apps / Plasma / Xwayland
          |
      [ waylandd ]
          |
      [ compd ]
          |
      [ displayd ] ---- [ lockd ]
          |
          +------------ [ sessiond ]
          |
  [ Linux Kernel ]
```

- **`displayd`**: Least-privileged broker for `DRM/KMS`, input, and seat / `DRM/KMS`、`input`、`seat` の最小特権 broker
- **`compd`**: Layout, composition, focus, and effects. Restartable / 配置、合成、フォーカス、効果。落ちても再起動可能
- **`waylandd`**: Manages client connections and object lifetimes / client 接続とオブジェクト寿命の保持
- **`lockd`**: Manages lock screen state and auth UI / ロック画面と認証 UI
- **`sessiond`**: Interfaces with lid, suspend, power, and polkit / lid、suspend、power、polkit 連携

---

## Document Directory / 文書一覧

All document links are relative / すべての文書リンクは相対パスです。

- [architecture.md](./architecture.md): Overall architecture and responsibility segmentation / 全体構造と責務分割
- [components.md](./components.md): Process roles and API boundaries / 各プロセスの役割と境界
- [failure-model.md](./failure-model.md): How things fail and how they recover / どう壊れ、どう復旧するか
- [plasma-integration.md](./plasma-integration.md): How to load on `KDE Plasma` / `KDE Plasma` にどう載せるか
- [roadmap.md](./roadmap.md): Phase planning, effort estimates, and exit criteria / 段階計画、工数、到達条件
- [design-memo.md](./design-memo.md): Repository policies and initial API boundaries / リポジトリ方針と初期 API 境界
- [repo-layout.md](./repo-layout.md): Directory structure of the repository / repository の階層構造
- [api-boundary.md](./api-boundary.md): Security boundaries and API surface / service 間の権限境界と初期 API 面
- [sequence-resume.md](./sequence-resume.md): Baseline sequence for suspend/resume recovery / suspend/resume 復帰時の基準シーケンス
- [ipc-format.md](./ipc-format.md): IPC envelopes and JSON message schemas / 初期 IPC envelope と JSON message 形状
- [crash-loop-policy.md](./crash-loop-policy.md): Watchdog restart policy and degraded mode criteria / watchdog の再起動と degraded mode の基準
- [desktop-profiles.md](./desktop-profiles.md): Guidelines for desktop profiles separated from the broker / GUI を broker 本体から分離し、profile として選択する方針
- [debian-integration.md](./debian-integration.md): Setup for permanent systemd-user integration on Debian / Debian へ user-space 常設統合する手順
- [linux-distro-socket.md](./linux-distro-socket.md): Linux socket targeting Debian/Ubuntu and Fedora/RHEL / Debian/Ubuntu と Fedora/RHEL 系を first-class support に絞った host socket
- [current-session-operation.md](./current-session-operation.md): Transition procedures to TUFF-Xwin within the current session / 現セッション限定で TUFF-Xwin を起動し、KDE/KWin から切り替える運用手順
- [xwin-screenshot-app.md](./xwin-screenshot-app.md): Specifications and design of the screenshot application / スクリーンショットアプリの設計と仕様

---

## Summary / ひとことで言うと

`Waybroker` is not about building a custom "Wayland Kernel", but rather micro-kernelizing the Wayland-generation display stack.

`Waybroker` は、「Wayland 専用カーネル」を作る話ではなく、「Wayland 世代の display stack をマイクロカーネル的に再分割する」話です。
