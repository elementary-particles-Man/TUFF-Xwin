# Debian Integration

この文書は Debian 固有の baseline です。major Linux 全体の受け口は [linux-distro-socket.md](/media/flux/THPDOC/Develop/TUFF-Xwin/docs/linux-distro-socket.md) を参照してください。

`TUFF-Xwin` を Debian へ常設注入する時の最小構成です。ここでは rootless な `systemd --user` 常駐を前提にし、次の 2 層に分けます。

- system-wide:
  `default.target = multi-user.target` にして display manager へ依存しない
- user-space:
  `displayd` / `waylandd` / `lockd` / `watchdog` / `sessiond` を `systemd --user` target で束ねる

## 実行環境方針

- 通常の install / start / recover 導線は `Bash` と `Rust` だけを前提にする
- `python3` は Debian 統合導線の必須要件にしない
- 将来の helper は可能な限り `Rust crate` か `Bash` に寄せる

## 何を入れるか

`scripts/install-user.sh` は次を行います。

- Rust toolchain がなければ `rustup` を user local に導入
- workspace を `release` build
- `~/.local/share/tuff-xwin/bin` へ service binary を配置
- `~/.local/bin` へ `tuff-xwin-start` / `tuff-xwin-stop` / `tuff-xwin-autostart` を配置
- `~/.config/systemd/user` へ user unit を配置
- `~/.config/tuff-xwin/session.env` を生成

## 起動モデル

### 手動起動

```bash
~/.local/bin/tuff-xwin-start host-wayland
```

### 一発復帰

TTY に落ちたあと、次の 1 コマンドで active profile を再選択して broker 群を再起動できます。

```bash
~/.local/bin/tuff-xwin-recover
```

必要なら profile を明示します。

```bash
~/.local/bin/tuff-xwin-recover host-wayland
```

### TTY1 からの自動起動

`~/.profile` で次のように `tuff-xwin-autostart` を呼ぶと、`tty1` にログインした直後に broker target を上げられます。実装は `start` ではなく `recover` を呼ぶので、前回 active profile が残っていればそれを優先して戻します。

```bash
if [ -x "$HOME/.local/bin/tuff-xwin-autostart" ]; then
    "$HOME/.local/bin/tuff-xwin-autostart"
fi
```

この repo では `tty1` / 非 GUI / 対話端末の時だけ起動する想定です。

## 既定 profile

`host-wayland` は Debian 常設向けの Wayland native profile です。

- broker 側:
  `displayd`, `waylandd`, `lockd`, `watchdog`, `sessiond`
- shell 側:
  `TUFF_XWIN_HOST_SHELL`
- panel/settings 側:
  `TUFF_XWIN_HOST_PANEL`, `TUFF_XWIN_HOST_SETTINGSD`

既定値は `gnome-shell --nested --wayland` です。ホスト側 command は `session.env` で差し替えます。

## 現時点の限界

- root 権限がないため `getty` autologin まではこの repo だけでは設定できない
- `host-wayland` は host shell command を受ける導線であり、GNOME/Plasma を完全分解して broker に再配線するところまではまだ行わない
- `displayd` / `waylandd` / `compd` は現在も stub/seed 実装を含むため、常用化には段階的な詰めが必要

それでも、Debian に TUFF-Xwin を「実行可能な基盤」として注入し、CUI 起点で broker 群を立ち上げるところまではこの構成で前進できます。
