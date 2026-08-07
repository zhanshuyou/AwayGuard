# AwayGuard

AwayGuard is a lightweight macOS menu bar app that automatically locks your screen when you step away
— no manual lock, no idle timers. It monitors Bluetooth proximity to your iPhone and triggers a lock
as soon as you're out of range, keeping your Mac secure without breaking your workflow.

## Known limitations

**The popover does not open over a full-screen app.** Clicking the menu bar icon while another app is
in full screen does nothing visible. The icon and the lock monitoring itself are unaffected — only the
panel is. Leave full screen and it opens normally.

This is a structural limitation rather than a missing setting, and the following have been measured
and ruled out: the panel already carries `CanJoinAllSpaces | FullScreenAuxiliary` and sits at
`NSStatusWindowLevel`, and with all three in place `isOnActiveSpace` still reports `false` over a
full-screen app. The window is shown, focused and fully opaque the whole time — just on a Space the
user is not looking at.

The cause is upstream of those knobs: opening the panel calls `set_focus`, which activates the app,
and activating is incompatible with staying on another app's full-screen Space. Fixing it means
replacing the plain `NSWindow` Tauri creates with a non-activating `NSPanel`
(`NSWindowStyleMaskNonactivatingPanel`), which is how menu bar apps normally do this. That touches
window creation and lifetime, the `windowEffects` vibrancy layer, the dismiss-on-focus-loss handling,
and keyboard focus for the controls in the panel, so it is deliberately left undone for now.
