# region-20261115 一次証跡

- `RESULT.md` / `RESULT.json`: 判定と残件
- `commands.txt`: 実行コマンド
- `runtime-status.log`: TUFF unit状態
- `retry-waylandd-journal.log`: Chrome接続、wl_region非再発、次の未実装要求
- `retry-chrome-*`: Chrome実行結果
- `cleanup.log` / `retry-cleanup.log`: TUFF専用cleanup
- `primary-start.status` / `primary-end.status`: primary状態
- `MANIFEST.json`: 全ファイルSHA-256/bytes

## PR37証跡修復

ローカルstateに残っていたPR37作業時の10ログを回収した。`build-and-tests.log`は指定名の原本が現存せず、manifestの`missing`へ記録した。回収ログは今回のxdg_toplevel実装検証結果ではなく、PR37既存実行の復旧証跡である。
