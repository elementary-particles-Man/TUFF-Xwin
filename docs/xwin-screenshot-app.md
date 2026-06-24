# Xwin Screenshot App / スクリーンショットアプリ

`xwin-screenshot` is the reference implementation of a lightweight screenshot application for the `TUFF-Xwin` system.
`xwin-screenshot` は、TUFF-Xwin の純正スクリーンショットアプリの最小実装です。

---

## Purpose / 目的

### English
- **Contain Graphic Failures**: Keep dialog and tool crashes separated from the main desktop session.
- **Protocol Validation**: Serve as a tool to verify the boundary contract with `displayd` via IPC.
- **Modular Responsibilities**: Separate hotkey triggering, system trays, UI interaction, formats, and file storing into independent modules.
- **Interactive User Workflow**: Ask user actions (Copy to clipboard or Save to folder) using native OS dialogs.

### 日本語
- **GUI 障害の隔離**: ダイアログやツールのクラッシュが、デスクトップセッション全体に影響しないようにします。
- **プロトコル検証**: `displayd` との境界を、IPC経由のクライアントコントラクトで検証できるようにします。
- **責務のモジュール化**: ホットキー、トレイ、UI、保存形式、保存先の制御をそれぞれ独立した小さなモジュールに分離します。
- **対話的ワークフロー**: OSネイティブのダイアログを使用し、撮影後に「コピー」か「保存」かを選択できるようにします。

---

## Configuration & CLI / 設定と引数

### Commands & Arguments / コマンドオプション
- `--backend <fake|isolated-displayd>`: Select capture client logic. / キャプチャクライアントロジックの選択。
- `--displayd-socket <PATH>`: Socket path to communicate with `displayd`. / `displayd` と通信するためのソケットパス。
- `--artifact-root <PATH>`: Root directory containing raw pixels artifacts. / 画像バッファ（RGBA）が書き出される一時ルートディレクトリ。
- `--save-dir <PATH>`: Output directory for saved images. / 保存先ディレクトリ。
- `--format <png|jpeg>`: Output file extension format. / 保存フォーマット。
- `--filename-template <TEMPLATE>`: Defaults to `xwin-{target}-{timestamp}` to prevent file overwrite on consecutive clicks. / ファイル名テンプレート。ミリ秒タイムスタンプを含めることで連打時の上書きを防止します。

---

## Actual Interactive Capturing Behavior / 実際の対話的キャプチャ挙動

### 1. Interactive Action Dialog / アクション選択ダイアログ
* **EN**: Once `xwin-screenshot` finishes writing the image, it spawns `kdialog` (KDE Plasma) or `zenity` (Gnome/Fallback) to ask the user: *"Do you want to copy the image to the clipboard, or save it to the folder?"*
* **JA**: キャプチャ完了後、`kdialog` (KDE) や `zenity` (Gnomeその他) が起動し、「撮影した画像をクリップボードにコピーしますか？（いいえを選択するとフォルダへ保存します）」と問いかけます。

### 2. Clipboard Integration / クリップボードコピー
* **EN**:
  - In a Wayland environment, it utilizes `wl-copy` (from the `wl-clipboard` package) to copy raw PNG/JPEG data into the system clipboard.
  - In an X11 environment, it falls back to `xclip` to load images into the selection buffers.
* **JA**:
  - Wayland環境では、`wl-clipboard` パッケージの `wl-copy` を使用して画像をクリップボードに格納します。
  - X11環境では、`xclip` を利用して画像をセレクションバッファへコピーします。

### 3. KDE Portal Selection (Fullscreen vs. Window) / KDEポータルでの全画面とウィンドウの選択
* **EN**:
  - When utilizing the Wayland portal (`--capture-method portal`), `displayd` requests `SourceType::Monitor | SourceType::Window` to let the user select screens or specific windows.
  - Due to XDG Desktop Portal specifications, **the physical monitor model name** (e.g., `HP Inc. HP 27f 4k (HDMI-A-1)`) represents the **Fullscreen (entire monitor screen)** target option. Selecting this button and clicking "Share" captures the full screen.
* **JA**:
  - Waylandポータルキャプチャ（`--capture-method portal`）を利用する際、`displayd` は `SourceType::Monitor | SourceType::Window` を要求します。これにより、画面とウィンドウの双方を切り替えて選択可能です。
  - ポータルの仕様上、ダイアログ内では「全画面」という固定テキストではなく、**接続中のモニターの型番・デバイス名**（例: `HP Inc. HP 27f 4k (HDMI-A-1)`）がボタンとして表示されます。このボタンを選択して「共有」を押すことで全画面が撮影されます。

---

## Architecture Boundaries / 境界設計

```text
xwin-screenshot (App CLI)
       |
  (Capture) -> [ DisplaydIpcCaptureClient ]
       |
   (IPC: CaptureOutput / OutputCaptured)
       |
   [ displayd ] -> (PipeWire / Portal / X11) -> writes raw RGBA8888 bytes
       |
  (Ingests raw buffer from allowed root)
       |
  (Encodes to PNG/JPEG -> triggers kdialog/zenity)
       |
  (Applies to wl-copy / xclip OR saves to directory)
```

## Known Limitations & Environment Conflicts / 既知の制限と環境競合

### English
- **KDE/Qt/Spectacle Coexistence Issue**: On some systems running KDE desktop components alongside standalone screenshot tools (such as Spectacle or flameshot), global shortcut daemon bindings (e.g. KGlobalAccel) might experience race conditions. This is an environment configuration/conflict issue under KDE/Qt and is NOT a defect or bug of the TUFF-Xwin framework.

### 日本語
- **KDE/Qt/Spectacle環境混在問題**: KDE デスクトップ環境の一部コンポーネントや、Spectacle・Flameshot などの単体スクリーンショットツールが混在する場合、グローバルショートカットデーモン（KGlobalAccelなど）のバインディング競合により、キーイベントの奪い合いや遅延が発生する場合があります。これは KDE/Qt 側の環境設定・競合に起因するものであり、TUFF-Xwin の不具合ではありません。
