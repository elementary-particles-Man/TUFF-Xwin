---
version: alpha
name: TUFF-Xwin Carbon & Lime
description: Visual identity and design system for the TUFF-Xwin screenshot app dialogs and potential control overlays.
colors:
  primary: "#1A1C1E"
  secondary: "#4A4D50"
  tertiary: "#98C379"
  neutral-bg: "#F5F6F7"
  neutral-dark: "#0F1011"
typography:
  title:
    fontFamily: Inter
    fontSize: 16px
    fontWeight: 600
    lineHeight: 1.25
  body:
    fontFamily: Inter
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: Space Grotesk
    fontSize: 12px
    fontWeight: 500
    lineHeight: 1.0
rounded:
  sm: 4px
  md: 8px
  lg: 12px
spacing:
  xs: 4px
  sm: 8px
  md: 16px
  lg: 24px
components:
  dialog:
    backgroundColor: "{colors.neutral-bg}"
    textColor: "{colors.primary}"
    rounded: "{rounded.lg}"
    padding: 24px
  button-primary:
    backgroundColor: "{colors.tertiary}"
    textColor: "{colors.neutral-dark}"
    rounded: "{rounded.md}"
    padding: 8px
  button-secondary:
    backgroundColor: "{colors.secondary}"
    textColor: "{colors.neutral-bg}"
    rounded: "{rounded.md}"
    padding: 8px
---

## Overview

TUFF-Xwin Carbon & Lime.
The design language is engineered to match a minimal, highly functional desktop environment layout. It blends technical obsidian neutrals with a modern, high-visibility lime green accent. The interface is optimized to feel robust, lightweight, and modern.

## Colors

The system uses a focused color palette designed for high legibility:
- **Primary (#1A1C1E):** Obsidian dark tone for major header texts and UI frame components.
- **Secondary (#4A4D50):** Slate gray for secondary borders, placeholders, and inactive buttons.
- **Tertiary (#98C379):** High-visibility Lime green. Used strictly for success highlights, action points, and active system notifications.
- **Neutral Background (#F5F6F7):** A bright, soft warm-gray that avoids eye strain.

## Typography

Minimal typography scale centered around sans-serif structures to match host systems:
- **Title (Inter, 16px, Semi-Bold):** Used for dialog headers and title panels.
- **Body (Inter, 14px, Regular):** General descriptive text and confirmation messages.
- **Label (Space Grotesk, 12px, Medium):** Technical metrics, shortcuts, and metadata labels.

## Layout

Dialogs and overlays must use standard logical padding grids:
- Outer margins of active dialog boxes should be `{spacing.lg}` (24px).
- Internal element margins and spacing are based on `{spacing.md}` (16px).
- Buttons and small inputs use `{spacing.sm}` (8px) for alignment.

## Elevation & Depth

No deep drop shadows. We use sharp, distinct flat borders:
- Active state elements have a 1px solid border of `{colors.secondary}`.
- Dialog shadows are flat, offset and hard-edged.

## Shapes

- UI Container panels and dialog boxes use `{rounded.lg}` (12px) to match modern application panels.
- Buttons, input boxes, and status badges use `{rounded.md}` (8px) for a slightly softer but structured look.

## Components

### Dialog Panel
A basic window container for confirmations and actions.
- Padding is `{components.dialog.padding}`.
- Uses `{components.dialog.backgroundColor}` for layout panels.

### Action Buttons
Primary buttons use dynamic coloring:
- Primary button background: `{components.button-primary.backgroundColor}`.
- Secondary button background: `{components.button-secondary.backgroundColor}`.

## Do's and Don'ts

- **Do:** Use `{colors.tertiary}` (lime green) strictly for primary call-to-actions, successful operations, or current selections.
- **Do:** Ensure high contrast for overlay text on buttons.
- **Don't:** Introduce generic primary colors (pure red or blue) that break the obsidian/lime scheme.
- **Don't:** Add rounded corners greater than `{rounded.lg}` to avoid looking overly cartoonish.
- **Do:** Highlight the final path of the saved PNG image clearly in notifications and command-line outputs for developer visibility.
- **Don't:** Attempt to display any action dialogs or user prompts unless the capture socket connection is fully established and valid.
