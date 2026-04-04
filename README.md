# TUFF-Xwin (Project Waybroker)

[日本語](#japanese) | [English](#english)

---

<a name="japanese"></a>
## 日本語 (Japanese)

### 概要
`TUFF-Xwin` は、次世代セキュアOS「TUFF-OS」向けに設計された、堅牢かつモジュール化された表示系アーキテクチャ **Waybroker** のリファレンス実装です。

従来のモノリシックなディスプレイスタック（KDE Plasma や GNOME 等）を、役割ごとに独立したマイクロサービス（`displayd`, `waylandd`, `compd`, `lockd`, `sessiond`, `watchdog`）に分割することで、単一障害点を排除し、表示系の一部がクラッシュしてもメインシステムやカーネルの継続稼働を保証します。

### 主要機能
- **モジュール型ディスプレイサーバ**: ハードウェア制御、プロトコル処理、ポリシー、認証 UI を分離。
- **マルチセッション・リカバリ**: セッションごとに隔離されたリカバリ制御を実現。
- **安全なパス管理**: `session_instance_id` のバリデーションとサニタイズによるパス安全性の確保。
- **Vulkan GPU 加速 (実験的)**: Vulkan™ API を活用した非同期 Compute Pipeline によるパケットフィルタリングや監査スキャンの高速化。
- **自己修復機構**: Watchdog による各コンポーネントの死活監視と自動復旧。

### Vulkan™ に関する告知
本プロジェクトは、利用可能な環境において Vulkan API 上に構築された計算バックエンドを使用することがあります。
Vulkan および Khronos ロゴは、The Khronos Group Inc. の登録商標です。
本プロジェクトは、The Khronos Group Inc. によって開発、配布、運営、認定、支援、推奨、または承認されたものではありません。本プロジェクトの設計、実装、動作、安全性に関する全ての責任は、本プロジェクト自体に帰属します。

詳細は [TRADEMARKS.md](TRADEMARKS.md) および [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) を参照してください。

### ビルドとインストール
Rust ツールチェーンがインストールされていることを確認してください。

```bash
# 全パッケージのビルド
cargo build --workspace --release

# テストの実行
cargo test --workspace
```

### クイックスタート
```bash
# 統合動作確認（スモークテスト）の実行
./scripts/run-integration-smoke.sh

# マルチセッション隔離テストの実行
./scripts/run-multi-session-recovery-isolation-smoke.sh
```

---

<a name="english"></a>
## English

### Overview
`TUFF-Xwin` is the reference implementation of **Waybroker**, a robust and modular display architecture designed for the next-generation secure operating system, TUFF-OS.

By splitting the traditional monolithic display stack into independent microservices (`displayd`, `waylandd`, `compd`, `lockd`, `sessiond`, and `watchdog`), TUFF-Xwin eliminates single points of failure, ensuring that the main system and kernel remain operational even if components of the display stack crash.

### Key Features
- **Modular Display Server**: Separation of hardware brokerage, protocol handling, policy, and auth UI.
- **Multi-Session Recovery**: Strictly scoped recovery orchestration per session instance.
- **Path-Safe Identity**: Robust validation and sanitization of `session_instance_id` for filesystem safety.
- **Vulkan GPU Acceleration (Experimental)**: Optional GPU compute backend built on the Vulkan™ API for high-performance packet filtering and audit scanning.
- **Self-Healing**: Watchdog-driven health monitoring and automated service recovery.

### Vulkan™ Notice
This project may use a compute backend built on the Vulkan API where available.
Vulkan and the Vulkan logo are registered trademarks of The Khronos Group Inc.
This project is NOT developed, distributed, operated, certified, supported, endorsed, or approved by The Khronos Group Inc. All responsibilities regarding design, implementation, behavior, and safety belong solely to this project.

For details, see [TRADEMARKS.md](TRADEMARKS.md) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

### Build & Install
Ensure you have the Rust toolchain installed.

```bash
# Build all packages
cargo build --workspace --release

# Run unit tests
cargo test --workspace
```

### Quick Start
```bash
# Run integration smoke test
./scripts/run-integration-smoke.sh

# Run multi-session recovery isolation test
./scripts/run-multi-session-recovery-isolation-smoke.sh
```

## Documentation
- [docs/architecture.md](docs/architecture.md) - High-level architecture
- [docs/session-instance-id-contract.md](docs/session-instance-id-contract.md) - Safety & Path Contracts
- [docs/status/FINAL_PASS_BASELINE_2026-04-04.md](docs/status/FINAL_PASS_BASELINE_2026-04-04.md) - Verification Evidence

## License
Dual-licensed under `MIT OR Apache-2.0`.
- [LICENSE-MIT](LICENSE-MIT)
- [LICENSE-APACHE](LICENSE-APACHE)
