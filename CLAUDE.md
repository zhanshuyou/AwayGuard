# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository state

This repository currently contains **no source code** — only `README.md` and an Apache-2.0 `LICENSE`.
There is no build system, dependency manifest, test suite, or CI configuration yet, so there are no
build/lint/test commands to document.

If you are adding the first code, the project scaffold (language, package manager, test runner) is an
open decision — confirm it with the user rather than assuming one. Once a scaffold exists, re-run
`/init` so this file describes the real commands and architecture.

## What the project is

AwayGuard is a macOS menu bar app that locks the screen automatically when the user walks away. It
monitors Bluetooth proximity to the user's iPhone and triggers a lock once the phone is out of range —
proximity-driven, not idle-timer-driven.

Implications for any implementation work here:

- It is a **macOS-only** app with a menu bar (status item) UI, not a windowed app.
- Core behavior depends on **Bluetooth (Core Bluetooth / RSSI proximity)** and on macOS
  **screen-lock APIs**, so the app needs Bluetooth entitlements and will hit permission prompts;
  much of it cannot be exercised in a headless test environment.
- RSSI is noisy and range thresholds are the central design problem — expect debouncing/hysteresis
  around the lock trigger rather than a single threshold comparison.
