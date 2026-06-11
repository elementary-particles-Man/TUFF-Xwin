# TUFF-Xwin Browser Surface Boundary

## Purpose

- `TUFF-Xwin` の `BrowserSurfaceBoundary` は、`Chrome` / `Chromium` / `Electron` / Chromium-based browser 由来の surface posture を passive に評価する。
- 役目は display / input / window / clipboard / drag-and-drop / file picker / GPU presentation / native messaging の境界を、browser runtime が存在する前提で整理することにある。
- `KAIRO` の `CisaKevBrowserRuntimeGate` は runtime の version / KEV / update / restart posture を扱い、`TUFF-Xwin` は browser surface が何に触れてよいかを評価する。
- この文書と module は browser の起動、URL の読み込み、JavaScript 実行、memory attach、exploit 再現を行わない。

## Decision contract

- `BrowserSurfaceBoundaryContext` は runtime family, posture, window role, surface state, clipboard, file boundary, input, GPU, extension/native posture, operator override を受ける。
- `BrowserSurfaceBoundaryDecisionState` は `NotApplicable` / `AllowSurface` / `ObserveSurface` / `ConstrainSurface` / `ClipboardQuarantine` / `FileBoundaryQuarantine` / `InputCaptureFailClosed` / `NativeMessagingFailClosed` / `GpuSurfaceFailClosed` / `KairoFailClosedPropagated` / `UnknownFailClosed` を持つ。
- `BrowserSurfaceBoundaryDecision` は `state`, `action`, `reason`, `findings` を返す。
- `action` は `Allow` / `Constrain` / `Quarantine` / `FailClosed` のいずれかに収束する。

## Current posture mapping

- `KAIRO` が `kairo_fail_closed` を返す場合、`TUFF-Xwin` は `KairoFailClosedPropagated` として fail closed する。
- `sensitive clipboard` 読み取りは明示 override なしでは quarantine に倒す。
- `file picker` と `drag-and-drop` は明示 override なしでは quarantine に倒す。
- `raw keyboard` / `input capture` / `pointer lock` は `observe` / `quarantine` / `unknown` posture では fail closed に倒す。
- `native messaging` / `external protocol handler` は明示 override なしでは fail closed に倒す。
- `shared GPU` / `direct scanout` / `screen mirroring` は `observe` / `quarantine` / `unknown` posture では fail closed に倒す。
- browser surface が安全側 posture かつ高リスク要求なしなら allow または constrain の低リスク側へ落とす。

## Non-goals

- `KAIRO` の version / KEV / update / restart logic を実装しない。
- browser process を起動しない。
- URL を開かない。
- JavaScript を実行しない。
- browser memory を inspect しない。
- browser profile / extension / policy / registry / sandbox state を mutate しない。
- webdriver / chromedriver / playwright / selenium を使わない。
