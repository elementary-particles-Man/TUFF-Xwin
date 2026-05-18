# Current Session Operation

## 目的

この文書は、既存の `KDE Plasma / KWin Wayland` セッション内で `TUFF-Xwin` broker 群を現セッション限定で起動し、起動確認後に `KDE / KWin` を停止して `TUFF-Xwin` 側へ表示制御を寄せる運用手順を記録する。

ここでの「現セッション限定」は、永続インストール済みの user unit を置き換えず、`/run/user/$UID/systemd/user` に一時 unit を生成することを意味する。再ログインや再起動後には、この一時 unit は残らない。

## 前提

作業対象 repository:

```bash
/mnt/thpdoc/Develop/TUFF-Xwin
```

通常は先に user-space 導入を済ませる。

```bash
./scripts/install-user.sh
```

ただし `scripts/tuff-xwin-current-session.sh` は、repo の release build 出力と `/usr/bin` も `PATH` に含めるため、既に binary がある環境では現セッション起動にも使える。

既定値:

```text
profile: host-wayland
session instance: current-session
runtime directory: $XDG_RUNTIME_DIR/waybroker
```

## 注意

`takeover` で `KDE / KWin` を停止すると、KDE 上で動いている端末、ブラウザ、Codex UI、通常の Wayland / Xwayland アプリは表示経路を失う可能性が高い。

切り替え後も状態確認を続ける場合は、別 TTY、SSH、または再起動後の Codex から確認する。

## 起動

現セッション限定の一時 unit を生成し、`TUFF-Xwin` broker 群を起動する。

```bash
cd /mnt/thpdoc/Develop/TUFF-Xwin
bash ./scripts/tuff-xwin-current-session.sh start
```

profile や session instance を明示する場合:

```bash
bash ./scripts/tuff-xwin-current-session.sh \
  --profile host-wayland \
  --session-instance-id current-session \
  start
```

`install-user.sh` 後は次の launcher でも同じ操作ができる。

```bash
~/.local/bin/tuff-xwin-current-session start
```

## 起動確認

```bash
bash ./scripts/tuff-xwin-current-session.sh status
```

期待する状態:

- `tuff-xwin-current-session.target` が `active`
- `displayd`, `waylandd`, `lockd`, `watchdog`, `sessiond`, `compd` が残っている
- この段階では `kwin_wayland` や `plasmashell` が残っていてもよい

`status` は `systemctl --user list-units` と対象プロセス一覧をまとめて表示する。

## KDE / KWin から切り替える

`TUFF-Xwin` の現セッション限定 target と各 service がすべて `active` であることを確認してから実行する。

```bash
bash ./scripts/tuff-xwin-current-session.sh takeover
```

このコマンドは次を順に行う。

- `TUFF-Xwin` current-session unit 群がすべて `active` か確認する
- `plasma-plasmashell.service`, `plasma-kwin_wayland.service`, `plasma-workspace*.target` などを停止する
- 残った `plasmashell`, `kwin_wayland`, `kwin_wayland_wrapper`, `kwin_x11`, `Xwayland`, `xwaylandvideobridge` を `TERM` 後に必要なら `KILL` する
- 最後に `status` を表示する

KWin / Plasma が user manager から再起動される環境では、現 user-manager lifetime 限定で関連 unit を runtime mask してから停止する。

```bash
bash ./scripts/tuff-xwin-current-session.sh --runtime-mask-plasma takeover
```

runtime mask は永続設定ではない。再ログインまたは user manager 再起動後には消える。

同じログイン中に runtime mask だけ外す場合:

```bash
bash ./scripts/tuff-xwin-current-session.sh unmask-plasma
```

## 切り替え後の確認

```bash
bash ./scripts/tuff-xwin-current-session.sh status
```

`TUFF-Xwin` 側だけが残り、`kwin_wayland`, `kwin_wayland_wrapper`, `plasmashell`, `Xwayland` が消えていることを確認する。

## 停止

現セッション限定の `TUFF-Xwin` を止める。

```bash
bash ./scripts/tuff-xwin-current-session.sh stop
```

一時 unit も削除する。

```bash
bash ./scripts/tuff-xwin-current-session.sh cleanup
```

## 恒久導線との違い

通常の常設運用は次を使う。

```bash
~/.local/bin/tuff-xwin-start host-wayland
~/.local/bin/tuff-xwin-stop
```

`tuff-xwin-current-session` は、既に Plasma/KWin が動いている現ログインセッションで安全確認を挟みながら移行するための限定導線であり、display manager から直接入る通常セッション entrypoint ではない。
