# TUFF-Xwin Age Assurance Browser Surface Boundary

## Purpose

- `TUFF-Xwin` の `AgeAssuranceBrowserSurfaceBoundary` は、`Chrome` / `Chromium` / `Electron` / Chromium-based browser 由来の age-assurance / identity-verifier surface を passive に評価する。
- 役目は browser/window/input/clipboard/file-picker/native-messaging の境界を、age assurance UI が存在する前提で整理することにある。
- `KAIRO` の `CisaKevBrowserRuntimeGate` は runtime の version / KEV / update / restart posture を扱い、`TUFF-Xwin` は browser surface が何に触れてよいかを評価する。
- この文書と module は age verification 自体を実装せず、URL の読み込み、JavaScript 実行、memory attach、browser profile mutation、exploit 再現を行わない。

## Decision contract

- `AgeAssuranceBrowserSurfaceBoundaryContext` は runtime family, posture, window role, surface state, clipboard, file boundary, extension/native posture, GPU posture, operator override を受ける。
- `AgeAssuranceBrowserSurfaceBoundaryDecisionState` は `NotApplicable` / `AllowSurface` / `ConstrainSurface` / `AgeSignalObserve` / `ClipboardQuarantine` / `FilePickerQuarantine` / `IdentityUploadFailClosed` / `BiometricPromptFailClosed` / `NativeMessagingFailClosed` / `ExternalProtocolFailClosed` / `ScreenCaptureFailClosed` / `KairoFailClosedPropagated` / `UnknownFailClosed` を持つ。
- `AgeAssuranceBrowserSurfaceBoundaryDecision` は `state`, `action`, `reason`, `findings` を返す。
- `action` は `Allow` / `Constrain` / `Quarantine` / `FailClosed` のいずれかに収束する。

## Current posture mapping

- `KAIRO` が `kairo_fail_closed` を返す場合、`TUFF-Xwin` は `KairoFailClosedPropagated` として fail closed する。
- `age_verifier_iframe` / `platform_age_signal_prompt` / `identity_provider_popup` / `browser_profile_switch_prompt` は、明示 override がなければ `AgeSignalObserve` に倒す。
- `clipboard read` は明示 override なしでは quarantine に倒す。
- `file picker` / `drag-and-drop` は明示 override なしでは quarantine に倒す。
- `government ID upload` は明示 override なしでは fail closed に倒す。
- `camera` / `biometric` prompt は明示 override なしでは fail closed に倒す。
- `native messaging` / `external protocol handler` は明示 override なしでは fail closed に倒す。
- `screen capture` / `mirroring` / `shared GPU` は fail closed に倒す。
- browser surface が安全側 posture かつ高リスク要求なしなら allow または constrain の低リスク側へ落とす。

## Non-goals

- `KAIRO` の version / KEV / update / restart logic を実装しない。
- browser process を起動しない。
- URL を開かない。
- JavaScript を実行しない。
- browser memory を inspect しない。
- browser profile / extension / policy / registry / sandbox state を mutate しない。
- age verification / identity verification / camera / biometric 実行をしない。
- webdriver / chromedriver / playwright / selenium を使わない。
