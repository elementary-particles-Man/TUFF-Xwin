# Linux Distro Socket

## 目的

`TUFF-Xwin` の「どんな OS でも使える socket」は、ここでは `major Linux distro` 向けの受け口を意味します。対象は次の 2 系統です。

- `Debian / Ubuntu` 系
- `Fedora / RHEL` 系

minor distro まで first-class support を広げるのは初期段階の仕事ではありません。まずは、実際に利用者が多く、`systemd --user` と `XDG` 前提を共有しやすい major 系に対して、起動・復旧・host shell 差分を閉じ込めます。

## socket の中身

この socket は network socket の話ではなく、`host distro` と `TUFF-Xwin` の境界層です。具体的には次を吸収します。

- distro family 判定
- package manager の違い
- host shell / panel / settings daemon の差し替え
- `systemd --user` への環境注入
- `session_instance_id` の family-aware 既定値

## 現在の前提

- `systemd --user` が使えること
- `bash` があること
- Rust toolchain を user local に入れられること
- profile は `host-wayland` を既定にすること

## 自動検出

`scripts/tuff-xwin-distro-socket.sh` は `/etc/os-release` を見て次を出します。

- `TUFF_XWIN_DISTRO_FAMILY`
- `TUFF_XWIN_DISTRO_ID`
- `TUFF_XWIN_PACKAGE_MANAGER`
- `TUFF_XWIN_SESSION_INSTANCE_ID`
- `TUFF_XWIN_HOST_SHELL`
- `TUFF_XWIN_HOST_PANEL`
- `TUFF_XWIN_HOST_SETTINGSD`

`install-user.sh` はこの結果を `session.env` に落とし、`tuff-xwin-start` / `tuff-xwin-recover` はそのまま `systemd --user` へ流します。

## 方針

- broker 本体は distro 非依存に保つ
- 差分は shell script と env file に寄せる
- `Debian-only` の文言や既定値は外す
- unsupported family は無理に通さず、major Linux 限定で明示的に落とす

## 既定の host command 選定

現在の既定検出は次の順で行います。

- shell:
  `gnome-shell --nested --wayland` -> `startplasma-wayland` -> `weston`
- panel:
  `plasmashell` -> `xfce4-panel`
- settings daemon:
  `gnome-settings-daemon` -> `kded6` -> `xfsettingsd`

これは「完全な desktop 自動構成」ではなく、major distro で最初の一歩を踏ませるための conservative な default です。
