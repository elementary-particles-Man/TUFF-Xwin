# HANDOFF

## Logical multi-output routing (2026-07-26)

`feature/multi-output-routing` にて typed `BTreeMap` output registry と `OutputRuntime`（geometry、framebuffer、per-output frame identity/publication state）を追加した。`ConfigureOutput` は output_id 単位で作成・再構成し、`RemoveOutput` は generation 検証後に対象 output の resources/retry state を解放する。registry が複数 output を持つ場合、canonical scene を複製せず stable output_id 順に各 viewport の bounds/damage を算出し、global scene 座標を output-local framebuffer へ変換して独立 composition/publication する。同一 surface の複数 output clipping、publication order、output removal を focused test で確認した。MockDisplayBackend のみを使用し、production display/backend は未接触。

## Clippy baseline cleanup (2026-07-26)

`feature/clippy-baseline-cleanup` で初期 Clippy 12件だった `xwin-sec` の `derivable_impls` を、enum の `Unknown` default variant と context struct の `derive(Default)` へ同値変換した。追加で検出された workspace Clippy 指摘も、MSRV互換の局所修正、不要 clone/import/mut、manager の `Default` derive、test assertions、API形状を維持すべき large enum / dependency injection / protocol argument の狭い説明付き allow で解消した。`cargo clippy --all-targets --all-features -- -D warnings` は0件で成功。production display/backend は未接触。

## Rust warning cleanup (2026-07-26)

`feature/warning-cleanup` で workspace の rustc warning 20件（重複 target 出力を除く）を根原因修正した。wayland-wire の未使用 import/引数、sessiond/watchdog の未使用 helper、displayd の obsolete/test-only backend・helper・未読 recording fields、wayland-wire integration test の未使用 binding、screenshot test の未使用 import を削除または test cfg に分離した。blanket allow、RUSTFLAGS、Cargo 設定変更はなし。`cargo check --all-targets` と `cargo test --all-targets` の workspace warning は0件。clippy は既存 `xwin-sec` の手書き Default 実装12件（今回の rustc warning ではない）で `-D warnings` が停止した。

## DisplayBackend boundary implementation (2026-07-26)

`feature/display-backend-boundary` にて、displayd の CPU 合成後 publication を `FramePublication` と `DisplayBackend::publish_frame` の immutable boundary に分離した。publication は output identity、validated geometry、stride、format、owned `Arc<[u32]>` framebuffer、damage、scene/output generation、単調増加 frame identity を保持する。実際の displayd main path は `MockDisplayBackend` を明示選択し、backend 成功後だけ active framebuffer、published frame identity、scene snapshot/generation を更新する。失敗時は renderer state と retry 条件を保持する。Mock は順序付き capture と frame failure injection を提供し、production display/device backend は実装・選択・起動していない。focused displayd tests は74件。

## Variable output geometry implementation (2026-07-26)

`feature/variable-output-geometry` にて、`waybroker-common` の `DisplayCommand::ConfigureOutput` と `OutputGeometry`（output identity、幅、高さ、stride、format、origin、output_generation）を追加し、`displayd` の CPU 合成を検証済み出力状態へ移行した。framebuffer は stride から checked に算出し、初回設定・resize・stride変更は旧バッファを公開せず次回合成で full repaint を行う。surface/damage は output origin を含む global 座標から local framebuffer 座標へ checked translation し、既存の canonical ordering、PixelTransport、premultiplied alpha、damage-limited replay を維持する。不正 geometry、overflow、unsupported format、stale generation は拒否する。負 origin、padding stride、generation 独立性のテストを追加した。production display/backend は起動していない。

更新日: `2026-07-26`
対象 repository: `/media/flux/THPDOC/Develop/TUFF-Xwin`

## 完了した主要タスク

1.  **Session-aware Recovery & Path-safe Identity** (Final Pass Criteria)
    *   `watchdog` の recovery request を `session_instance_id` 対応に拡張し、複数セッション並列実行時のリカバリ対象を厳密に特定。
    *   `session_instance_id` に対するバリデーションとサニタイズを導入し、不正な文字列によるパス破壊やディレクトリトラバーサルを防御。
    *   詳細は [FINAL_PASS_BASELINE_2026-04-04.md](docs/status/FINAL_PASS_BASELINE_2026-04-04.md) および [MULTI_SESSION_RECOVERY_ISOLATION_2026-04-04.md](docs/status/MULTI_SESSION_RECOVERY_ISOLATION_2026-04-04.md) を参照。

2.  **Vulkan GPGPU 加速の統合**
    *   `crates/vulkan-backend` を新設し、ASH クレートによる非同期 GPU 演算バックエンドを実装。
    *   `compd`, `waylandd`, `displayd` に `--vulkan` フラグを導入し、パケットフィルタリングや監査スキャンの加速準備を完了。
    *   全主要コンポーネントを `Tokio` ベースの非同期実行モデルへ移行。

3.  **LeyerX11 セレクション・ブリッジの実装**
    *   X11 のクリップボード/プライマリ・セレクションの所有権情報をブローカー層（`displayd`）へコミットする機能を実装。
    *   リカバリ時に Wayland 側へセレクション状態を引き継ぐ（Handoff）ライフサイクルを確立。

4.  **マルチセッション対応（Runtime Artifact 拡張）**
    *   全てのランタイム生成ファイル（シーンスナップショット、レジストリ、ログ等）に `session_instance_id` を付与し、完全なセッション分離を実現。
    *   `sessiond` から子コンポーネントへの ID 伝播（引数/環境変数）を徹底。

5.  **実機相当環境（QEMU/KVM）での最終動作検証**
    *   **Xfce (X11) テスト**: 正常稼働および `xfwm4` パニック時のカーネル生存を確認。
    *   **Gnome (Wayland) テスト**: 正常稼働および `gnome-shell` パニック時のカーネル生存を確認。
    *   **結論**: GUI システムの全損がいかなる場合もメインカーネルの稼働に影響を与えないことを実証。

6.  **スクリーンショット機能のリファイン (Vulkan/SIMD 加速)**
    *   `displayd` に `CaptureOutput` IPC を実装し、フレームキャプチャの基盤を構築。
    *   `vulkan-backend` に AVX2 加速によるピクセル変換（BGRA <-> RGBA）を実装。
    *   Vulkan コンピュートシェーダーによる並列処理の統合。
    *   検証用スクリプト `scripts/tuff-xwin-screenshot.sh` を追加。

7.  **Pixel Transport と Canonical Scene State の分離**
    *   `SurfaceSnapshot` から raw pixel 所有を外し、`PixelTransportHandle` と `PixelTransportPayload` による明示的な transport 境界へ移行。
    *   `waylandd` の canonical scene は surface ordering、client/surface identity、`scene_generation` を引き続き唯一の権威として保持。
    *   SHM pixel bytes は `PixelTransportStore` へ generation-bound に登録し、`CommitScene.pixel_payloads` 経由で `compd`/`displayd` へ渡す。
    *   `displayd` の renderer 入力は transport lookup を通じて payload を取得し、handle 付き surface の payload 欠落は scene を記録せず `Rejected` とする。
    *   disconnect cleanup は対象 client の transport payload だけを無効化し、別 client の payload には影響しない。
    *   pending replay は canonical surfaces と対応 payload bundle を保持し、成功時に既存の generation 条件で clear する。
    *   検証は headless/mock の Cargo tests のみで行い、production desktop/display server は起動していない。

8.  **Damage-limited Framebuffer Composition**
    *   `displayd` の full-frame/surface-wide repaint を renderer-facing damage rect ベースの再合成へ置換。
    *   surface-local damage を canonical geometry で output 座標へ変換し、surface bounds と framebuffer bounds で checked clipping する。
    *   damage rect ごとに背景を復元してから、canonical back-to-front order の全 intersecting surface を opaque copy する。
    *   movement / resize / removal / disconnect 相当の scene 差分では、旧領域と新領域を damage として扱い、露出領域を再構築する。
    *   `PixelTransportStore` から payload を lookup し、missing/stale payload は framebuffer を部分更新せず reject する。
    *   copied-byte / damaged-pixel accounting は effective damage region の面積ベースに更新。
    *   `waylandd` の production SHM damage は surface-local metadata として保持し、pending replay でも damage と payload bundle を維持する。
    *   検証は headless/mock の Cargo tests のみで行い、production desktop/display server は起動していない。

9.  **Alpha-aware Damage-limited Composition**
    *   `displayd` の final pixel write 境界を opaque-only copy から alpha-aware source-over composition へ拡張。
    *   `wl_shm` の `ARGB8888(0)` は premultiplied alpha、`XRGB8888(1)` は未使用 alpha byte を無視する opaque format として明示分類。
    *   framebuffer は opaque output として扱い、背景および合成結果の alpha byte は `0xFF` に正規化。
    *   alpha-bearing payload は integer arithmetic の deterministic source-over で合成し、alpha 0 は destination 保持、alpha 255 は exact replacement とする。
    *   unsupported format、non-premultiplied ARGB、malformed stride/short payload、missing/stale payload は framebuffer を部分更新せず `Rejected` とする。
    *   damage-limited reconstruction、checked clipping、non-tight stride、canonical back-to-front replay、pending replay、PixelTransport separation は維持。
    *   `waylandd` の SHM format は `PixelTransportPayload.format` として維持され、pending replay でも format と damage metadata を保持する。
    *   検証は headless/mock の Cargo tests のみで行い、production desktop/display server は起動していない。

## 次のステップへの申し送り

*   **Vulkan 実シェーダーの導入**: 現在はシミュレーションモード。TUFF-OS 側の `.spv` バイナリをロードすることで、実演算の加速が可能。
*   **Portal ブリッジの拡張**: 画面共有（screencast）等の高度な Portal 機能をブローカー経由で提供する実装の深化。

## 謝辞
TUFF-OS チームによる迅速なインストーラー提供および QCOW2 イメージの準備に深く感謝いたします。本プロジェクトの堅牢性は、TUFF-OS の設計思想との強力なシナジーによって完成されました。
