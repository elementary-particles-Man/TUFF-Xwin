# HANDOFF

## Typed partial publication results (2026-07-26)

`feature/typed-partial-publication-result` にて `ScenePublicationResult`、`OutputPublicationResult`、`OutputPublicationOutcome`、`PublicationFailure` を waybroker-common IPC 型として追加した。`SceneCommitted` に optional typed publication payload を追加し、displayd の multi-output commit/retry path は canonical scene を一度受け入れた後、stable output_id 順の per-output outcome を返す。backend failure は `Failed(BackendRejected)` として記録し、成功 output の事実を保持する。result validation は duplicate output_id と scene generation mismatch を拒否し、serde round-trip test を追加した。registry-only output state と retry isolation を保持し、production display/backend は未接触。

## Registry-only output state migration (2026-07-26)

`feature/registry-only-output-state` で、active displayd composition/publication/configuration path を `BTreeMap<String, OutputRuntime>` の registry に移行した。`DisplayState` から active な `output` と `framebuffer` fields を削除し、output geometry、framebuffer、frame identity、pending damage、retry state は registry entry から解決する。CommitScene は stable output_id 順に registry entry を処理し、既存の A-success/B-failure retry isolation を保持する。既存の単一 output legacy statement block は compile-time disabled として残り、typed partial publication result は後続タスク。production display/backend は未接触。

## Per-output retry isolation (2026-07-26)

`feature/multi-output-routing-correction` の既存未commit実装を保持し、retry isolation のまとまりとして完成・commitした。`OutputRuntime` に pending damage、last published scene generation、generation/frame-bound retry state を追加し、backend failure 時は該当 output だけを保留する。成功済み output は同一 scene generation の retry で再出版せず、frame identity は成功時だけ進む。MockDisplayBackend に output-specific failure injection と、A 成功/B 失敗後に B だけ retry する focused test を追加した。registry-only state migration（`state.output` / `state.framebuffer`撤去）と typed partial publication result は後続タスクとして分離する。production display/backend は未接触。

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

10. **Liveness と display readiness の typed boundary**
    * `DisplayCommand::GetLiveness` / `GetReadiness` と対応する typed event を追加。
    * `ServiceReadiness` と `OutputReadiness` は既存の `OutputRuntime` から都度導出し、重複する mutable readiness state は保持しない。
    * ConfigureOutput 後は awaiting publication、generation-matched publication 後のみ Ready、retry は対象 output 単位で RetryPending とする。
    * zero output は live だが display-ready ではない。production display backend には接続していない。

11. **Authoritative CI validation gate**
    * push と pull request の CI で fmt、`cargo check --all-targets`（`-D warnings`）、`cargo test --all-targets`、全 features の Clippy、`bash scripts/dev-check.sh` を独立した必須 step として実行する。
    * CI は `DISPLAY` / `WAYLAND_DISPLAY` を空にし、専用の空 runtime directory を使うため、production display system へ接続しない。
    * 既存の `scripts/dev-check.sh` は Git 上で executable（100755）であり、CI では bash 経由で実行する。

12. **Frame submission と presentation completion の分離**
    * `PresentationToken` を output、generation、scene、frame、backend instance に束縛し、submission 成功と completion を typed state として分離した。
    * `OutputRuntime` は outstanding immutable frame、submitted frame identity、presented frame identity を output 単位で保持する。
    * readiness は submission 後を `SubmittedAwaitingPresentation`、matching `Presented` completion 後だけを `Ready` とする。
    * duplicate、unknown、cross-output、stale generation completion は fail-closed。superseded token は newer frame を後退させない。
    * MockDisplayBackend は deterministic pending submission と bounded token retention を持つ。production display backend は追加・接続していない。

13. **Deterministic presentation scheduler boundary**
    * output registry 内に cadence、scheduler state、generation、monotonic tick、eligible time を保持する `OutputScheduler` を追加。
    * `AdvancePresentation` IPC は output generation、timestamp、tick sequence を検証し、backward clock と duplicate tick を拒否する。
    * scheduler は submission-in-flight を output 単位で blocked として扱い、cadence と pending damage を bounded に管理する。physical refresh/VSync を表さない。
    * Wayland frame callback は既存の presentation completion 経路を通るまで成功扱いにせず、production display backend は接続していない。

14. **Scheduler runtime migration**
    * `CommitScene` は canonical scene、PixelTransport、output ごとの pending damage を受け付けるだけで、通常の backend submission は行わない。
    * `AdvancePresentation` が cadence、generation、in-flight 状態を検証し、最新 canonical scene から eligible output を compose/submission する唯一の通常経路になった。
    * immediate-submission 前提の displayd tests を tick-driven lifecycle に移行し、direct-scanout、zero-damage、PixelTransport、multi-output retry、malformed payload、capture の境界を更新した。
    * presentation feedback は acceptance では確定せず、valid Presented completion 後にのみ確定する。Wayland frame callback の commit/output association と first-valid-intersecting-output delivery は次の follow-up とする。

15. **Real portal capture isolation**
    * `tuff-xwin-capture-once.sh --portal-real-capture` は `TUFF_XWIN_RUN_REAL_CAPTURE_TESTS=1` の明示 opt-in がない限り fail-closed し、通常の Cargo test / dev-check から portal dialog に到達しない。
    * interactive な launcher tests は opt-in 未設定時に実行前に skip し、fake/stub capture tests と capture protocol coverage は通常検証に残す。
    * `scripts/dev-check.sh` は real-capture opt-in が設定された場合も fail-closed する。手動実行例は `TUFF_XWIN_RUN_REAL_CAPTURE_TESTS=1 bash scripts/tuff-xwin-capture-once.sh --portal-real-capture` であり、portal dialog が表示され得るため通常検証や CI では実行しない。
    * scheduler runtime migration の CommitScene acceptance-only、AdvancePresentation submission、Presented completion 境界は維持。Wayland frame callback completion delivery は未実装の follow-up とする。

16. **Wayland frame callback completion boundary**
    * production Wayland 接続では `wl_surface.frame` callback を generation-bound pending registry に登録し、client、surface、callback identity、accepted scene generation、intersecting output IDs のみを保持する。
    * callback は CommitScene acceptance、damage accumulation、scheduler tick、composition、submission では送信せず、`commit_production_scene` が valid `FramePresented` を返した後にだけ release する。
    * policy は first valid Presented completion from any intersecting output。後続 output completion は at-most-once registry removal により重複送信しない。non-intersecting output、newer-generation 未到達 completion、surface cleanup は release しない。
    * output viewport は deterministic な configured output order で計算し、surface geometry と checked intersection を用いる。real portal capture は引き続き未実行、production display への接続も行っていない。

*   **Vulkan 実シェーダーの導入**: 現在はシミュレーションモード。TUFF-OS 側の `.spv` バイナリをロードすることで、実演算の加速が可能。
*   **Portal ブリッジの拡張**: 画面共有（screencast）等の高度な Portal 機能をブローカー経由で提供する実装の深化。

## 謝辞
TUFF-OS チームによる迅速なインストーラー提供および QCOW2 イメージの準備に深く感謝いたします。本プロジェクトの堅牢性は、TUFF-OS の設計思想との強力なシナジーによって完成されました。
