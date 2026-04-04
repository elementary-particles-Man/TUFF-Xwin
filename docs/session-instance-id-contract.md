# Session Instance ID Contract

## 1. 仕様

`session_instance_id` は、Waybroker の各コンポーネントが特定のセッション（supervisor instance）を識別し、そのランタイム・アーティファクトを分離するために使用される識別子です。

### 許可される文字 (Allowlist)
- `[A-Za-z0-9._-]` (英数字、ドット、アンダースコア、ハイフン)

### 長さ制限
- 最小: 1 文字
- 最大: 128 文字

### 禁止事項
- `.` および `..` 単体での指定（ディレクトリトラバーサル防止）。
- NUL バイト、制御文字、`/`, `\`, `:` などのパス区切り文字。

## 2. 正規化と安全性 (Sanitization)

システムは入力された ID に対して以下の処理を行います。
- 不許可文字はすべて `_` (アンダースコア) に置換されます。
- `.` または `..` が指定された場合、`_.` または `_..` に変換されます。
- 128文字を超える分は切り捨てられます。
- 空文字の場合は `default` が使用されます。

## 3. アーティファクト・パス規則

全てのランタイム・アーティファクトは `WAYBROKER_RUNTIME_DIR` (通常は `/run/user/$UID/waybroker/`) 配下に生成されます。

パス生成ロジックは以下の通り固定されます：
`{RUNTIME_DIR}/session-{sanitized_id}-{artifact_name}.json`

例:
- ID: `demo.01`, Artifact: `launch-state`
  -> `session-demo.01-launch-state.json`
- ID: `../evil`, Artifact: `watchdog-report`
  -> `session-.._evil-watchdog-report.json`

## 4. 通信プロトコルにおける扱い

- **Legacy 互換**: 旧バージョンの IPC メッセージ（`session_instance_id` フィールド欠如）を受信した場合、`legacy-single-session` という固定 ID が適用されます。
- **新経路**: 以降に追加される全てのリカバリ要求（`WatchdogCommand::Restart` 等）および状態通知は、明示的に `session_instance_id` を含めなければなりません。
- **Watchdog**: 監視対象の特定に ID を必須とし、ID が一致しないメッセージや状態更新は原則として破棄または Resync 要求の対象となります。
