# Components / コンポーネント

## displayd

### English
#### Roles
- Owns `DRM/KMS` control master.
- Manages output modes and display connectors.
- Receives events from `libinput`.
- Coordinates `seat` and `VT` transitions.
- Exposes secure screenshot/screencast interfaces (PipeWire / Portal).

#### State Managed
- Current KMS settings.
- Connector / CRTC / Plane status registry.
- List of connected input devices.
- Last successfully committed frame memory state.

#### Non-Roles
- Stacking/window layout policy.
- Desktop UI controls.
- Lockscreen authentication rules.

#### What happens if displayd crashes?
- Display output is lost, but the kernel and other user-space processes survive. Recovery should be achievable via `systemd --user`, VT switcher, or SSH.

---

### 日本語
#### 役割
- `DRM/KMS` の所有
- output mode 管理
- `libinput` の受信
- `seat` と `VT` の制御
- secure display path の最低限の提供（PipeWire / Portal 連携）

#### 持つべきもの
- KMS state
- connector / crtc / plane 情報
- 入力デバイス列挙
- 最後に成功した frame state

#### 持つべきでないもの
- ウィンドウ配置ポリシー
- デスクトップ UI
- lockscreen の認証ロジック

#### 死んだらどうなるか
- 表示は失うが、kernel と他の process は生きる。`systemd --user`、VT、SSH から復旧可能であることを目標にする。

---

## waylandd

### English
#### Roles
- Terminates native Wayland client connections.
- Exposes Wayland global interfaces.
- Tracks surface and buffer resource lifetimes.
- Provides backing layer for selection/clipboard and DnD.

#### Design Rationale
- Decouples client connections from the active compositor rendering logic. If `compd` or `Xwayland` crashes, client sockets and surface registrations survive.
- The prototype tracks a `surface registry snapshot` (including current clipboard/selection owners). After `compd` restarts, it queries this registry, matches against `displayd`'s hardware status, reconstructs the active scene, and seamlessly hands focus back.

---

### 日本語
#### 役割
- Wayland client 接続端点
- global object の提供
- surface と buffer の寿命追跡
- clipboard / selection / DnD の基盤

#### 設計意図
- `compositor` が死んでも、client 接続そのものは維持しやすくする。`KWin` や `Mutter` を一度落としたら全 client が道連れになる現状を避ける。
- 現時点の試作では、`waylandd` は `surface registry snapshot` を IPC で返せます。snapshot には clipboard / primary selection owner も含みます。`compd` restart 後は `displayd` の last scene とこの registry を突き合わせ、生きている surface だけで scene を再構成し、dangling owner があれば focus へ handoff します。

---

## compd

### English
#### Roles
- Constructs the desktop scene graph.
- Manages active input focus state.
- Controls window stacking/layering.
- Performs window decorations and transitions.
- Enforces window rules and placement.

#### Positioning
- The core of KWin/Mutter logic. Contains the vast majority of desktop layout code but operates completely unprivileged.

#### What happens if compd crashes?
- The watchdog automatically restarts it. Since `waylandd` keeps client connections active and `displayd` retains the hardware surface state, `compd` can reconstruct the visual arrangement immediately upon reboot.

---

### 日本語
#### 役割
- scene graph の構築
- focus 管理
- stacking order
- decoration と effect
- window rule と placement

#### 位置づけ
- これは `KWin-core` や `Mutter-core` に近い存在です。見た目や UX の大半はここに載りますが、特権は持たせません。

#### 死んだらどうなるか
- `watchdog` が再起動する。`waylandd` 和 `displayd` が残っていれば、接続維持、最終画面保持、clipboard / selection owner の再整列ができます。

---

## lockd

### English
#### Roles
- Displays the lock screen window interface.
- Performs authentication logic (PAM interaction).
- Signals successful unlocking to the stack.

#### Rationale for Isolation
- Locking locking logic inside the main compositor means a failure in the password dialog or auth library crashes the entire display system. Decoupling ensures auth failures are handled safely as application errors.
- The desktop profile manager treats `lockd` as a standard background security overlay component rather than a core compositor thread.

---

### 日本語
#### 役割
- ロック画面の表示
- 認証 UI
- PAM 連携
- unlock 成功の通知

#### 分離理由
- lockscreen を `compositor` 本体に抱え込むと、「席を外した時の認証」の失敗が「画面系すべての停止」に化けやすいからです。
- `Wayland native` の session profile では、lock UI を毎回 profile component としてぶら下げなくてもよい形を目指します。最低限の desktop skeleton は `shell` / `panel` / `settings-daemon` / `applet` に集中させ、認証 UI 自体は broker-owned `lockd` が持つ前提に寄せます。

---

## sessiond

### English
#### Roles
- Receives hardware events like lid close/open.
- Issues system suspend/resume requests.
- Manages idle/sleep policies.
- Coordinates polkit/dbus session permissions.
- Manages desktop profiles and active system state.

#### Rationale for Isolation
- Isolates complex daemon policies (like `PowerDevil` or `gnome-settings-daemon`) from core graphics pipeline processes.
- Acts as a desktop profile manager (`demo-x11`, `openbox-x11`, etc.) ensuring components are spawned and fallback mechanisms are applied correctly during failures.

---

### 日本語
#### 役割
- lid close / open
- suspend / resume の要求
- idle policy
- `polkit` 連携
- power key などの session policy
- desktop profile の選択と active state 管理

#### 分離理由
- `PowerDevil` や `gnome-settings-daemon` 的な都合を display server の心臓部に持ち込まないためです。
- 同時に、`xfce` や軽量 WM のような GUI profile 選択を broker 本体へ焼き込まず、user が入れ替えられるようにするためでもあります。

---

## Xwayland

### English
#### Roles
- Accommodates legacy X11 applications.
- Provides rootless window framing.
- Bridges X11 selections to Wayland clipboards.

#### Policy
- Ensures a crash in `Xwayland` does not affect native Wayland clients. The repository contains `LeyerX11/` (`layerx11-common` and `x11bridge`) acting as a compatibility island to safely commit rootless layout scenes.

---

### 日本語
#### 役割
- 既存 X11 アプリの収容
- rootless window 提供
- X selection と Wayland clipboard の橋渡し

#### 方針
- `Xwayland` が死んでも、Wayland native app は死なない構成を保つことが重要です。
- 現時点の repository では、その compatibility island の試作置き場として `LeyerX11/` を追加し、rootless window state を `displayd` へ commit する最小 bridge を置いています。

---

## watchdog

### English
#### Roles
- Monitors display services (`compd`, `lockd`, `sessiond`, `waylandd`).
- Detects and suppresses crash loops.
- Triggers degraded/fallback desktop profiles during recovery failures.

#### Policy Constraints
- The watchdog is a lightweight supervisor dedicated strictly to graphics recovery. It is not designed to replace `systemd` or init (PID 1).

---

### 日本語
#### 役割
- `compd`、`lockd`、`sessiond`、`waylandd` の監視
- crash loop 抑制
- degraded mode / degraded-profile への切り替え

#### 注意点
- watchdog は権限を持ちすぎてはいけません。`PID 1` の代用品にせず、display stack 専用の supervisor として扱います。

---

## Boundary Summary / 境界のまとめ

```text
displayd = hardware broker
waylandd = protocol broker
compd    = policy and composition
lockd    = auth UI
sessiond = power and session policy
Xwayland = compatibility island
watchdog = recovery control
```
