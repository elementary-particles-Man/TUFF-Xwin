# TUFF-Xwin 導入と Takeover 手順

この文書は、`TUFF-Xwin` を既存のデスクトップ環境の上に入れ、現セッションで表示制御を引き継ぐための導入手順をまとめたものです。

ここでの前提は次の通りです。

- 恒久的な OS 設定はすでに行われている
- `root` / `sudo` は使わない
- `systemd --user` を使う
- 既存の KDE Plasma, KWin, Xwayland, Chrome を壊さない
- `TUFF-Xwin` の変更対象は repo 内に限定する

## 1. 何を恒久設定として入れるか

導入時に必要なのは、OS 全体の大改造ではなく、user session 側の常駐基盤です。

- `~/.config/systemd/user/` へ TUFF-Xwin の user unit を配置する
- `~/.local/share/tuff-xwin/bin/` に service binary を配置する
- `~/.local/bin/` に起動用ラッパーを置く
- `~/.config/tuff-xwin/session.env` に session 固有の環境を置く
- `XDG_RUNTIME_DIR/waybroker/` を runtime artifact の置き場として使う

これらは `./scripts/install-user.sh` が行う前提です。

## 2. 現セッション take over の意味

`current-session` は、いまログインしている既存セッションに対して一時的な unit を作り、TUFF-Xwin の broker 群を起動する導線です。

- 永続 unit を切り替えない
- 再ログイン後に残る前提にしない
- `systemd --user` の runtime unit を使う

起動の基本は次です。

```bash
cd /mnt/thpdoc/Develop/TUFF-Xwin
bash ./scripts/tuff-xwin-current-session.sh start
```

確認は次です。

```bash
bash ./scripts/tuff-xwin-current-session.sh status
```

期待状態は `displayd`, `waylandd`, `lockd`, `watchdog`, `sessiond`, `compd` が active であることです。

## 3. 実際に必要だった事実ベースの前提

現セッションの確認では、次の runtime artifact が見えました。

- `/run/user/1000/waybroker/displayd.sock`
- `/run/user/1000/waybroker/waylandd.sock`
- `/run/user/1000/waybroker/lockd.sock`
- `/run/user/1000/waybroker/watchdog.sock`
- `/run/user/1000/waybroker/sessiond.sock`
- `/run/user/1000/waybroker/compd.sock`
- `/run/user/1000/wayland-0`
- `/run/user/1000/wayland-1`

この状態は、TUFF-Xwin が少なくとも現セッション内で Wayland 側の制御点を持ち、`waylandd` が client 接続を受けた事実と整合します。

## 4. Takeover の一般化

KDE Plasma を使っている場合の実例では、`KWin` と `plasmashell` を止めました。  
ただし、ここは KDE 固有に固定しません。一般化すると次の考え方です。

- まず TUFF-Xwin 側を active にする
- 次に既存 compositor / shell / workspace manager を止める
- 既存 compositor が user manager により再起動される場合は runtime mask を使う
- `wayland-*.lock` の残骸があれば stale 扱いで整理する

KDE Plasma の例としては、次のユニット群が対象でした。

- `plasma-kwin_wayland.service`
- `plasma-plasmashell.service`
- `plasma-workspace*.target`

他の compositor の場合は、同等の display manager / compositor / shell unit に置き換えます。

## 5. 競合で問題になった点

今回の実機調査で確認した競合は 2 つです。

1. `compd` の IPC accept 経路で、接続異常を fatal に扱うと service が落ちる
2. `wayland-1.lock` が残っていると `waylandd` が起動ループする

これに対して repo 側では次を入れました。

- accept の recoverable error を panic せず継続する
- stale な `wayland-N.lock` を socket 不在なら削除して再 bind する
- `wayland-N` が使えないときは `wayland-N+1`, `wayland-N+2` を試す

## 6. 導入後の確認項目

以下を順に見ると、導入が壊れていないかを切り分けやすくなります。

```bash
bash ./scripts/tuff-xwin-current-session.sh status
systemctl --user --no-pager --full status tuff-xwin-current-session-waylandd.service tuff-xwin-current-session-sessiond.service
pgrep -a -u "$USER" 'kwin_wayland|Xwayland|plasmashell|chrome|displayd|waylandd|sessiond|compd|lockd|watchdog'
journalctl --user --no-pager -n 200 | grep -Ei 'tuff|xwin|waylandd|compd|panic|coredump|lock already|socket_bound|client_connected|stale_lock'
```

見るべき事実は次の通りです。

- `waylandd` が active である
- `wayland-*.lock` が stale のまま残っていない
- `compd` が coredump していない
- `KDE / KWin / Xwayland / Chrome` の生存を壊していない

## 7. 停止と復帰

`TUFF-Xwin` を止めるときは、現セッションの runtime unit だけを止めます。

```bash
bash ./scripts/tuff-xwin-current-session.sh stop
```

復帰確認は次です。

```bash
bash ./scripts/tuff-xwin-current-session.sh cleanup
```

KDE Plasma を維持したまま戻す場合は、`kwin_wayland`, `plasmashell`, `Xwayland` が生きていることを確認します。

## 8. 補足

この文書は、KDE Plasma での現場例を含みますが、実装の意図は汎用 compositor に対して同じです。  
停止対象の unit 名とプロセス名だけを環境ごとに差し替えればよく、手順の骨格は同じです。
