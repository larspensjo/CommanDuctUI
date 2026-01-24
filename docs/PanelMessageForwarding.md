# Panel Message Forwarding: Parent Notification Family

## Problem

Panels act as intermediate containers between the main window and child controls. Win32 sends notification messages to a control's **direct parent**, not to the top-level window. When a button lives inside a panel, messages like `WM_DRAWITEM` go to the panel, not the main window where our handlers are.

The current implementation uses an explicit allowlist of forwarded messages. This is fragile: each new feature that introduces a message type silently fails until someone remembers to update the list.

## Solution: Forward the Complete Parent Notification Family

Win32 defines a finite, stable set of "parent notification" messages — messages the OS sends to a control's parent on behalf of that control. This set is defined by the Win32 API and does not grow when application features are added.

### The Complete Set

```rust
const PARENT_NOTIFICATION_MESSAGES: &[u32] = &[
    // Command and notification
    WM_COMMAND,         // Button clicks, edit notifications, accelerators
    WM_NOTIFY,          // Common control notifications (TreeView, ListView, etc.)
    WM_PARENTNOTIFY,    // Child creation/destruction, mouse clicks

    // Owner-draw
    WM_DRAWITEM,        // Owner-drawn buttons, listboxes, menus
    WM_MEASUREITEM,     // Owner-drawn item measurement (variable-height items)
    WM_DELETEITEM,      // Owner-drawn item deletion (cleanup)
    WM_COMPAREITEM,     // Owner-drawn sorted listbox comparison

    // Control coloring
    WM_CTLCOLORBTN,     // Button background
    WM_CTLCOLOREDIT,    // Edit control background
    WM_CTLCOLORSTATIC,  // Static/label background
    WM_CTLCOLORLISTBOX, // Listbox background
    WM_CTLCOLORSCROLLBAR, // Scrollbar background
    WM_CTLCOLORDLG,     // Dialog background

    // Scrolling from child controls
    WM_HSCROLL,         // Horizontal scroll from child scrollbar/trackbar
    WM_VSCROLL,         // Vertical scroll from child scrollbar/trackbar
];
```

### Why This Set Is Stable

These messages are part of the Win32 API contract. Microsoft has not added new parent notification messages in decades — modern controls use `WM_NOTIFY` with different notification codes instead. Forwarding this complete set means:

- No maintenance burden when adding new application features
- No silent failures from forgotten forwarding
- No risk of the set growing unexpectedly

### What NOT to Forward

Messages the panel needs to handle itself or that are not parent notifications:

- `WM_PAINT` / `WM_ERASEBKGND` — panel's own painting (note: `WM_ERASEBKGND` is currently forwarded for panel background support; this should be reconsidered in favor of the panel painting its own background directly)
- `WM_SIZE` / `WM_MOVE` — panel's own geometry
- `WM_CREATE` / `WM_DESTROY` — panel's own lifecycle
- `WM_TIMER` — belongs to whichever window set the timer
- Mouse/keyboard messages — belong to the control that received them

### Implementation

Replace the current allowlist:

```rust
// BEFORE (fragile, incomplete)
if (msg == WM_COMMAND
    || msg == WM_CTLCOLOREDIT
    || msg == WM_CTLCOLORSTATIC
    || msg == WM_DRAWITEM
    || msg == WM_NOTIFY
    || msg == WM_ERASEBKGND)
```

With a match against the complete family:

```rust
// AFTER (complete, stable)
fn is_parent_notification(msg: u32) -> bool {
    matches!(msg,
        WM_COMMAND
        | WM_NOTIFY
        | WM_PARENTNOTIFY
        | WM_DRAWITEM
        | WM_MEASUREITEM
        | WM_DELETEITEM
        | WM_COMPAREITEM
        | WM_CTLCOLORBTN
        | WM_CTLCOLOREDIT
        | WM_CTLCOLORSTATIC
        | WM_CTLCOLORLISTBOX
        | WM_CTLCOLORSCROLLBAR
        | WM_CTLCOLORDLG
        | WM_HSCROLL
        | WM_VSCROLL
    )
}
```

### WM_ERASEBKGND Consideration

The current code forwards `WM_ERASEBKGND` from panels to the parent for panel background painting. This is a workaround — `WM_ERASEBKGND` is not a parent notification message. A cleaner approach would be for the panel to paint its own background by looking up its applied style directly, rather than delegating to the parent. This would allow removing `WM_ERASEBKGND` from the forwarding list entirely.

## References

- [WM_DRAWITEM](https://learn.microsoft.com/en-us/windows/win32/controls/wm-drawitem)
- [WM_NOTIFY](https://learn.microsoft.com/en-us/windows/win32/controls/wm-notify)
- [WM_CTLCOLORSTATIC](https://learn.microsoft.com/en-us/windows/win32/controls/wm-ctlcolorstatic)
- MFC source: `CWnd::OnWndMsg` and `CWnd::ReflectLastMsg` in `wincore.cpp`
- WTL: `REFLECT_NOTIFICATIONS()` macro in `atlcrack.h`
