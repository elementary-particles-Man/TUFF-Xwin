# fix-20261114 一次証跡

- [README.md](README.md): 実装、検証、実機Chrome結果、保全範囲。
- [commands.txt](commands.txt): 実行コマンドと終了コード。
- [waylandd-journal.txt](waylandd-journal.txt): Chrome接続後のwaylandd観測結果。
- [MANIFEST.json](MANIFEST.json): SHA-256、bytes、missing、redaction情報。

Chromeの`wl_seat` opcode 2障害は解消したが、次の未対応`wl_region` opcode 1で終了したため、GUI描画PASSとは扱わない。
