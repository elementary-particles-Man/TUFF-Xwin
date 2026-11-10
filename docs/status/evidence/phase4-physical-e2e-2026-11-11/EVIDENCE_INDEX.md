# Phase 4 physical E2E 一次証跡 index

収録日: 2026-11-11 JST
対象 baseline: `ee3a678ae90832763590683d48cd4de81de0c728`（PR #32 merge）

このディレクトリは、既存のローカル run root に保存されていたテキスト形式の一次証跡を、元の相対パスを保って tracked 収録したものです。要約への再圧縮はしていません。全収録ファイルは [MANIFEST.json](MANIFEST.json) の sha256/bytes と照合できます。

## 収録一覧

- [initial_e2e/](initial_e2e/): 初回 Phase 4 の RESULT、software verification、smoke、live-topology logs
  - [RESULT.json](initial_e2e/reports/RESULT.json)
  - [RESULT.md](initial_e2e/reports/RESULT.md)
  - [RESULT-retest.json](initial_e2e/reports/RESULT-retest.json)
  - [RESULT-retest.md](initial_e2e/reports/RESULT-retest.md)
  - [software-verification.log](initial_e2e/logs/software-verification.log)
  - [fix-verification.log](initial_e2e/logs/fix-verification.log)
  - [live-topology-fixed-10x.log](initial_e2e/logs/live-topology-fixed-10x.log)
  - [live-topology failure runtime logs](initial_e2e/logs/live-topology/)
- [phase4_retest/](phase4_retest/): 再テスト報告
  - [RESULT-retest.md](phase4_retest/RESULT-retest.md)
- [physical_20261110/](physical_20261110/): 残4項目の実行結果、コマンド環境、終了状態
  - [RESULT-physical.json](physical_20261110/reports/RESULT-physical.json)
  - [RESULT-physical.md](physical_20261110/reports/RESULT-physical.md)
  - [02-software-regression.log](physical_20261110/evidence/02-software-regression.log)
  - [02-smokes.log](physical_20261110/evidence/02-smokes.log)
  - [03-real-devices.txt](physical_20261110/evidence/03-real-devices.txt)
  - [11-final-state.txt](physical_20261110/evidence/11-final-state.txt)
- [REDACTIONS.json](REDACTIONS.json): 実際の秘密値置換の記録（0件）

## 直接確認できる判定

- PR #32後の live-topology 10回連続 PASS と `production_client_error=0`
- software regression 各コマンドの実行結果・終了コード
- DRM pageflip / authenticated uinput / TUFF Chrome GUI / portal real capture の BLOCKED理由
- physical hotplug の NOT_RUN理由
- 元KWin・既存session・元primary worktreeの保全結果

## 未収録

- バイナリ、socket、browser profile、EDID本体、秘密鍵は収録していない。
- EDID本体はコピーせず、取得元と SHA-256 のみ `physical_20261110/evidence/03-real-devices.txt` に記録している。
- `MANIFEST.json` の `missing` は空。既知の指定結果ファイルは全て収録済み。
