# Crash Loop Policy / クラッシュループ・ポリシー

## Goal / 目的

### English
`Waybroker` does not assume components never crash. Instead, it defines strict reboot escalation rules, specifying where to cease recovery and drop into a **degraded mode** or trigger a **degraded profile fallback**.

### 日本語
`Waybroker` は「死なないこと」を前提にしません。その代わり、どこで再起動をやめて degraded mode に落ちるか、あるいは degraded プロファイルへのフォールバックを適用するかを定義します。

---

## Scope / 対象範囲

### English
This policy applies to all services monitored and supervised by the `watchdog`:
- `compd` (Compositor and Stacking)
- `lockd` (Lockscreen and Authentication)
- `sessiond` (Session and Profile Manager)
- `waylandd` (Wayland Socket Broker)
- `displayd` (Hardware KMS/Input Broker)

### 日本語
この文書の対象は `watchdog` が監視する service です。
- `compd`
- `lockd`
- `sessiond`
- `waylandd`
- `displayd`

---

## Policy Details / ポリシー詳細

### compd
- **1st crash within 30s**: Immediate restart. / 30秒以内に1回目: 即時再起動。
- **3rd crash within 30s**: Request disabling rendering effects (shadows, blur). / 30秒以内に3回目: エフェクト無効化を要求。
- **5th crash within 30s**: Suspend Xwayland integration; drop to a minimal composition scene. / 30秒以内に5回目: Xwayland連携を一時停止して最小sceneに降格。
- **7th crash within 30s**: Notify the user of a degraded desktop profile; hold user session state for recovery. / 30秒以内に7回目: ユーザーに degraded デスクトップを通知し、通常セッションを維持したまま復帰待ちへ。

### lockd
- **1st crash within 30s**: Immediate restart. / 30秒以内に1回目: 即時再起動。
- **3rd crash within 30s**: Fallback to `blank-only` screen lock (no auth dialog). / 30秒以内に3回目: `blank-only`（認証ダイアログなしのブランク画面）へ降格。
- **5th crash within 30s**: Terminate the lock screen interface and report auth failure to `sessiond`. / 30秒以内に5回目: ロックUIを停止し、認証失敗を `sessiond` へ通知。

### sessiond
- **1st crash within 30s**: Immediate restart. / 30秒以内に1回目: 即時再起動。
- **3rd crash within 30s**: Demote power policies to conservative no-op defaults. / 30秒以内に3回目: 電源・サスペンドポリシーを安全な no-op デフォルト値へ降格。
- **5th crash within 30s**: Log power policy failure while keeping core graphic pipelines intact. / 30秒以内に5回目: ポリシーエラーをログに記録しつつ、表示パイプラインは維持する。

### waylandd
- **1st crash within 30s**: Immediate restart. / 30秒以内に1回目: 即時再起動。
- **2nd crash within 30s**: Escalate as a critical system incident; prioritize holding client connection handles. / 30秒以内に2回目: クリティカルインシデントとして処理。クライアント接続の維持を最優先として対応。

### displayd
- **1st crash within 30s**: Watchdog requests classification of failure. / 30秒以内に1回目: watchdog が原因分類を要求。
- **Drivers/Kernel failure suspected**: Do not loop restart. Transition user interface to SSH/VT fallback guides. / カーネルやドライバ由来の障害と判定された場合は再起動ループを防止。ユーザーへは VT/SSH 経由での手動復旧案内を優先表示。

---

## Technical Implementation (Rust) / 技術実装 (Rust)

### 1. Active Profile Fallback IPC / アクティブプロファイル・フォールバック
* **EN**: The fallback process is executed via the `watchdog` communicating with `sessiond --serve-ipc --manage-active` to automatically adjust profile contexts (`demo-x11-degraded` / `demo-x11-crashy`).
* **JA**: `watchdog -> sessiond` の IPC 通信を介して実行されます。`sessiond --serve-ipc --manage-active` により、アクティブなプロファイル状態を動的に `demo-x11-degraded` などへ切り替えます。

### 2. Event-Driven Health Stream / イベント駆動型ヘルスストリーム
* **EN**: `sessiond` streams components' launch states to `watchdog` dynamically:
  - Initial connection delivers a complete snapshot of all running states.
  - Subsections are updated incrementally using `UpdateLaunchState` packets.
  - If the watchdog detects a sequence discrepancy, it triggers a `ResyncLaunchState` request.
* **JA**: `sessiond` から `watchdog` へコンポーネント起動状態がストリーミング送信されます：
  - 初回接続時にすべてのコンポーネント状態のフルスナップショットを送信。
  - 以降の変更は `UpdateLaunchState` による増分パッチで通知。
  - シーケンス番号（`generation` / `sequence`）に乖離を検知した場合、`ResyncLaunchState` でフル同期を再要求します。

---

## Log Format / ログ出力規約

```text
watchdog role=compd crash_loop_count=3 action=disable-effects reason=segfault
```

---

## IPC Mapping / Rust定義

* **Rust Module**: [ipc.rs](../crates/waybroker-common/src/ipc.rs) (`WatchdogCommand`, `HealthState`, `LaunchState`, `UpdateLaunchState`, `ResyncLaunchState`)
