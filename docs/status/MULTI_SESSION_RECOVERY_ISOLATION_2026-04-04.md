# Multi-Session Recovery Isolation Evidence

## 1. 検証目的
`WatchdogCommand::Restart` リクエストが `session_instance_id` によって厳密にスコープされ、他の並列セッションに影響を与えないこと、および不当な ID に対するパス安全性が確保されていることを実証する。

## 2. 実行環境
- OS: Linux (Ubuntu 22.04 on WSL2)
- Commit SHA: `a4aaa67f9152e454f512419942c6f82feb5d9a83`
- コマンド: `./scripts/run-multi-session-recovery-isolation-smoke.sh`

## 3. 実行結果 (Evidence)

### セッション Alpha へのリカバリ要請
```text
==> Sending recovery request for session-alpha...
python3 msg = { "session_instance_id": "alpha", "role": "compd", ... }
```

### 生成されたアーティファクト
- `session-alpha-watchdog-recovery-compd.json` (生成を確認)
- `session-beta-watchdog-recovery-compd.json` (存在しないことを確認 -> 隔離成功)

### セッション Alpha アーティファクト内容
```json
{
  "role": "compd",
  "reason": "simulated failure for alpha",
  "requested_by": "sessiond",
  "unix_timestamp": 1775298089,
  "action": "restart-request-accepted",
  "status": "pending"
}
```

### パス安全性検証 (Insecure ID Injection)
- 送信 ID: `../evil`
- 生成ファイル: `session-.._evil-watchdog-recovery-compd.json`
- 判定: 不正文字 `/` が置換され、ディレクトリトラバーサルが発生せずに安全なパスに固定されている。

## 4. 結論
Watchdog におけるリカバリ要求のセッション隔離およびパス安全性は **PASS**。
E2E レベルでの安全仕様準拠を確認した。
