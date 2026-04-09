# Waybroker

`Waybroker` は、既存の `Debian Linux kernel` をそのまま使いながら、`KDE Plasma` や `GNOME` の表示系を故障分離するための構想メモです。内部コードネームとして `TUFF-Xwin` を併記します。目的は新しいカーネルを作ることではなく、`display/input/session` の責務を分割し、`compositor` が落ちても OS 全体や作業セッションを巻き込まない構成を作ることです。

## なぜ必要か

現行の `KWin Wayland` や `GNOME Shell` は、平常時の統合度は高い一方で、障害時には責務が密結合しすぎています。表示、入力、ウィンドウ管理、ロック画面、電源連携が一体化しているため、単一障害点が太すぎます。

`Waybroker` は、この問題を「Wayland 専用カーネル」で解くのではなく、`userspace の最小特権 broker` と `再起動可能な display stack` で解く前提に立ちます。

## 設計目標

- `Debian kernel` はそのまま使う
- `KDE Plasma` や `GNOME` の上位 UX は極力維持する
- `compositor` が落ちても kernel や session 全体は落とさない
- `lockscreen`、`power management`、`policy` を display server 本体から分離する
- `X11` 的な故障分離を、Wayland 世代の構成に持ち込む

## 非目標

- Linux kernel の書き換え
- 全デスクトップ環境の完全互換を初期段階から保証すること
- すべての kernel deadlock や driver bug を userspace だけで救うこと
- 既存 `KWin` や `Mutter` を無改造でそのまま使うこと

## 要点

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
  [ Debian Linux Kernel ]
```

- `displayd`: `DRM/KMS`、`input`、`seat` の最小特権 broker
- `compd`: 配置、合成、フォーカス、効果。落ちても再起動可能
- `waylandd`: client 接続とオブジェクト寿命の保持
- `lockd`: ロック画面と認証 UI
- `sessiond`: lid、suspend、power、polkit 連携

## 文書一覧

- [architecture.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/architecture.md): 全体構造と責務分割
- [components.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/components.md): 各プロセスの役割と境界
- [failure-model.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/failure-model.md): どう壊れ、どう復旧するか
- [plasma-integration.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/plasma-integration.md): `KDE Plasma` にどう載せるか
- [roadmap.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/roadmap.md): 段階計画、工数、到達条件
- [design-memo.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/design-memo.md): リポジトリ方針と初期 API 境界
- [repo-layout.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/repo-layout.md): repository の階層構造
- [api-boundary.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/api-boundary.md): service 間の権限境界と初期 API 面
- [sequence-resume.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/sequence-resume.md): suspend/resume 復帰時の基準シーケンス
- [ipc-format.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/ipc-format.md): 初期 IPC envelope と JSON message 形状
- [crash-loop-policy.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/crash-loop-policy.md): watchdog の再起動と degraded mode の基準
- [desktop-profiles.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/desktop-profiles.md): GUI を broker 本体から分離し、profile として選択する方針
- [debian-integration.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/debian-integration.md): Debian へ user-space 常設統合する手順

## ひとことで言うと

`Waybroker` は、「Wayland 専用カーネル」を作る話ではなく、「Wayland 世代の display stack をマイクロカーネル的に再分割する」話です。
