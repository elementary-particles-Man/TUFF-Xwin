# TUFF-Xwin Screenshot Isolated VM Test Plan

## Baseline

- main HEAD: `8c6362c258f9c11a47cdc6f2bff090b6e34597d3`
- この文書は隔離VM/テスト機検証計画であり、実装ではない
- この文書作成時点では VM 起動・実OS操作・実displayd.sock接続を行わない

## Purpose

- 実 displayd.sock 接続前に隔離環境の手順を固定する
- 現用OS・現用Wayland session・現用displayd.sockを保護する
- GUI panic と kernel panic を連動させない設計原則を、検証手順側にも適用する
- 失敗してもVMまたはテスト機だけで閉じることを目的にする

## Current Repo State Before VM Work

- xwin-screenshot fake backend は PNG/JPEG 保存可能
- isolated-displayd backend は明示socket pathとartifact rootで動作する
- ArtifactRoot hardening 済み
- test harness displayd 済み
- config file support 済み
- AGENTS.md workspace hygiene rule は main 固定済み
- 現時点で実displayd.sockには接続していない

## Allowed Isolated Environment

- VMまたは専用テスト機のみ
- 現用デスクトップ環境では実施しない
- SSHまたはCUI復旧導線を先に確保する
- snapshotまたは復元可能な状態を先に確保する
- 検証用ユーザーを分離する
- 検証用runtime directoryを明示pathで分離する

## Still Forbidden Before Explicit Integration Phase

- 現用OSのdisplayd.sock接続
- 現用Wayland session接続
- 現用XDG_RUNTIME_DIR探索
- 現用DISPLAY参照
- 現用WAYLAND_DISPLAY参照
- 実global hotkey登録
- 実system tray登録
- 実DRM/KMS/PipeWire/input device接続
- 本番Chrome実プロセス判定
- Chrome/V8 exploit検出
- ブラウザsandbox再実装

## VM Phase Candidate Steps

1. repo clone / checkout only
2. cargo fmt/check/test
3. xwin-screenshot --backend fake のCLI確認
4. xwin-screenshot --backend isolated-displayd を harness / tempdir socket で確認
5. config file flow を tempdirのみで確認
6. test harness displayd E2Eを確認
7. ここまで通ってから、隔離環境内の実displayd.sock接続を別phaseで検討する

## Preconditions Before Real displayd.sock Test

- workspace clean
- origin/main同期
- CUI/SSH recovery path確保
- VM snapshotまたはテスト機復旧手順確保
- 実displayd.sock pathを明示指定する
- runtime自動探索を使わない
- policy hook が transport 前に走ることを再確認する
- 失敗時ログとartifactを回収できるようにする

## Failure Handling

- panicではなくResult errorとして扱う
- displayd failure は screenshot側へ伝播する
- policy deny は socket接続前に止める
- artifact contract mismatch は保存前に拒否する
- VM側GUIが壊れてもCUI/SSHで回収する
- kernel panicが起きた場合は検証停止し、実OS統合へ進まない

## Acceptance Criteria

- repo内 cargo fmt --check PASS
- repo内 cargo check --workspace PASS
- repo内 cargo test --workspace PASS
- fake backend CLI PASS
- isolated-displayd harness CLI PASS
- config file fake flow PASS
- config file isolated-displayd flow PASS
- 実OS非干渉
- 現用displayd.sock非接続
- workspace clean維持

## Non-Goals

- この文書で実装を行わない
- この文書でVMを起動しない
- この文書でFedora/Ubuntu等のイメージを取得しない
- この文書で実displayd.sockへ接続しない
- この文書で実Wayland sessionへ接続しない
- この文書で実global hotkey/trayを実装しない
- この文書でDRM/KMS/PipeWire/input deviceへ触らない

## Permanent Workspace Rule

- 作業開始前に workspace clean を確認する
- 未コミット差分が出た場合は次作業へ進まず専用ブランチで回収する
- dirty file を理由にrepo移動・repo削除をしない
- cleanではない状態を放置して次フェーズへ進まない
- AGENTS.md のような運用差分も正式差分として扱う
