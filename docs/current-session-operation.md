# Current Session Operation

## 目的

この文書は、既存の `KDE Plasma / KWin Wayland` セッション内で `TUFF-Xwin` broker 群を現セッション限定で起動し、その後 `KDE / KWin` を落として `TUFF-Xwin` のみを残すための運用手順を記録する。

ここでの「現セッション限定」は、永続インストールされた `/usr/lib/systemd/user/tuff-xwin-*.service` や `~/.config/systemd/user` を変更せず、`/run/user/$UID/systemd/user` に一時unitを置くことを意味する。再ログインや再起動後には、この一時unitは残らない。

## 前提

作業対象 repository:

```bash
/mnt/thpdoc/Develop/TUFF-Xwin
```

現行の package 配置:

```text
/usr/bin/displayd
/usr/bin/waylandd
/usr/bin/compd
/usr/bin/lockd
/usr/bin/sessiond
/usr/bin/watchdog
```

runtime directory:

```bash
/run/user/$(id -u)/waybroker
```

profile:

```text
host-wayland
```

session instance:

```text
current-session
```

## 注意

この手順で `KDE / KWin` を停止すると、KDE上で動いている端末、ブラウザ、Codex UI、通常のWayland/Xwaylandアプリは表示経路を失う可能性が高い。

そのため、切り替え後に状態確認を続ける場合は、別TTY、SSH、または再起動後のCodexに、 `/home/flux/tuff-xwin_after_start_codex_instructions.md` を読ませる。

## TUFF-Xwinを現セッション限定で起動する

一時unitディレクトリとruntime directoryを作る。

```bash
mkdir -p "/run/user/$(id -u)/systemd/user" "/run/user/$(id -u)/waybroker"
```

以下のunitを `/run/user/$(id -u)/systemd/user` に配置する。

### `tuff-xwin-current-session.target`

```ini
[Unit]
Description=TUFF-Xwin current-session runtime target
Wants=tuff-xwin-current-session-displayd.service tuff-xwin-current-session-waylandd.service tuff-xwin-current-session-lockd.service tuff-xwin-current-session-watchdog.service tuff-xwin-current-session-sessiond.service
```

### `tuff-xwin-current-session-displayd.service`

```ini
[Unit]
Description=TUFF-Xwin current-session display broker
PartOf=tuff-xwin-current-session.target

[Service]
Type=simple
Environment=WAYBROKER_RUNTIME_DIR=/run/user/1000/waybroker
ExecStart=/usr/bin/displayd --session-instance-id current-session
Restart=on-failure
RestartSec=1
```

### `tuff-xwin-current-session-waylandd.service`

```ini
[Unit]
Description=TUFF-Xwin current-session Wayland broker
PartOf=tuff-xwin-current-session.target
After=tuff-xwin-current-session-displayd.service
Requires=tuff-xwin-current-session-displayd.service

[Service]
Type=simple
Environment=WAYBROKER_RUNTIME_DIR=/run/user/1000/waybroker
ExecStart=/usr/bin/waylandd --serve-ipc --require-displayd --session-instance-id current-session
Restart=on-failure
RestartSec=1
```

### `tuff-xwin-current-session-lockd.service`

```ini
[Unit]
Description=TUFF-Xwin current-session lock broker
PartOf=tuff-xwin-current-session.target

[Service]
Type=simple
Environment=WAYBROKER_RUNTIME_DIR=/run/user/1000/waybroker
ExecStart=/usr/bin/lockd --serve-ipc
Restart=on-failure
RestartSec=1
```

### `tuff-xwin-current-session-watchdog.service`

```ini
[Unit]
Description=TUFF-Xwin current-session watchdog
PartOf=tuff-xwin-current-session.target

[Service]
Type=simple
Environment=WAYBROKER_RUNTIME_DIR=/run/user/1000/waybroker
ExecStart=/usr/bin/watchdog --serve-ipc
Restart=on-failure
RestartSec=1
```

### `tuff-xwin-current-session-sessiond.service`

```ini
[Unit]
Description=TUFF-Xwin current-session session supervisor
PartOf=tuff-xwin-current-session.target
After=tuff-xwin-current-session-displayd.service tuff-xwin-current-session-waylandd.service tuff-xwin-current-session-lockd.service tuff-xwin-current-session-watchdog.service
Requires=tuff-xwin-current-session-displayd.service tuff-xwin-current-session-waylandd.service tuff-xwin-current-session-lockd.service tuff-xwin-current-session-watchdog.service

[Service]
Type=simple
Environment=WAYBROKER_RUNTIME_DIR=/run/user/1000/waybroker
Environment=TUFF_XWIN_PROFILE=host-wayland
Environment=TUFF_XWIN_SESSION_INSTANCE_ID=current-session
Environment=TUFF_XWIN_REPO_ROOT=/mnt/thpdoc/Develop/TUFF-Xwin
Environment=TUFF_XWIN_PROFILES_DIR=/mnt/thpdoc/Develop/TUFF-Xwin/profiles
Environment=PATH=/usr/bin:/bin:/home/flux/.local/bin
ExecStartPre=/usr/bin/sessiond --repo-root /mnt/thpdoc/Develop/TUFF-Xwin --profiles-dir /mnt/thpdoc/Develop/TUFF-Xwin/profiles --select-profile host-wayland --write-selection --session-instance-id current-session
ExecStart=/usr/bin/sessiond --repo-root /mnt/thpdoc/Develop/TUFF-Xwin --profiles-dir /mnt/thpdoc/Develop/TUFF-Xwin/profiles --serve-ipc --manage-active --spawn-components --notify-watchdog --session-instance-id current-session
Restart=on-failure
RestartSec=1
```

unitを読み込み、古いsocketを消して起動する。

```bash
systemctl --user daemon-reload
systemctl --user stop tuff-xwin-current-session.target >/dev/null 2>&1 || true
rm -f "/run/user/$(id -u)/waybroker"/*.sock
systemctl --user start tuff-xwin-current-session.target
```

起動確認:

```bash
systemctl --user is-active \
  tuff-xwin-current-session.target \
  tuff-xwin-current-session-displayd.service \
  tuff-xwin-current-session-waylandd.service \
  tuff-xwin-current-session-lockd.service \
  tuff-xwin-current-session-watchdog.service \
  tuff-xwin-current-session-sessiond.service
```

期待値はすべて `active`。

## runtime確認

```bash
find "/run/user/$(id -u)/waybroker" -maxdepth 2 -type f -o -type s -o -type d 2>/dev/null | sort | xargs -r ls -la
```

代表的な期待値:

```text
compd.sock
displayd.sock
lockd.sock
sessiond.sock
watchdog.sock
waylandd.sock
session-current-session-active-profile.json
session-current-session-launch-state.json
session-current-session-surface-registry.json
```

## KDE / KWinを停止してTUFF-Xwinだけを残す

`TUFF-Xwin` の現セッション限定targetがすべて `active` であることを確認してから実行する。

```bash
systemctl --user stop \
  plasma-plasmashell.service \
  plasma-kwin_wayland.service \
  app-org.kde.xwaylandvideobridge@autostart.service \
  plasma-workspace-wayland.target \
  plasma-workspace.target \
  plasma-core.target || true
```

残存プロセスを落とす。

```bash
pkill -TERM -x plasmashell || true
pkill -TERM -x kwin_wayland || true
pkill -TERM -x kwin_wayland_wrapper || true
pkill -TERM -x Xwayland || true
pkill -TERM -x xwaylandvideobridge || true
```

数秒待って残っていれば強制終了する。

```bash
sleep 2
pkill -KILL -x plasmashell || true
pkill -KILL -x kwin_wayland || true
pkill -KILL -x kwin_wayland_wrapper || true
pkill -KILL -x Xwayland || true
pkill -KILL -x xwaylandvideobridge || true
```

## 切り替え後の確認

```bash
systemctl --user status tuff-xwin-current-session.target \
  tuff-xwin-current-session-displayd.service \
  tuff-xwin-current-session-waylandd.service \
  tuff-xwin-current-session-lockd.service \
  tuff-xwin-current-session-watchdog.service \
  tuff-xwin-current-session-sessiond.service --no-pager
```

```bash
ps -eo pid,ppid,stat,etimes,cmd | rg '(/|\b)(displayd|waylandd|compd|lockd|sessiond|watchdog|kwin_wayland|plasmashell|Xwayland)(\b|/)'
```

`TUFF-Xwin` 側だけが残り、`kwin_wayland`, `plasmashell`, `Xwayland` が消えていることを確認する。

## 停止

現セッション限定の `TUFF-Xwin` を止める。

```bash
systemctl --user stop tuff-xwin-current-session.target
```

一時unitを消す。

```bash
rm -f /run/user/$(id -u)/systemd/user/tuff-xwin-current-session*
systemctl --user daemon-reload
```

