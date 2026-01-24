# Plan: Paint Routing Refactor for ControlKind-Aware Styling

## Problem Summary

Read-only EDIT controls can emit WM_CTLCOLORSTATIC, which the current styling
path treats like a label (transparent background). This causes stale pixels
when the control scrolls (e.g., duplicated last line). The platform already
tracks ControlKind, but paint routing is still message-driven rather than
control-driven. This mismatch is fragile and can reappear in other controls.

## Goals

- Route paint/styling based on ControlKind first, message second.
- Ensure read-only EDIT controls are always painted as opaque edit controls.
- Keep the Win32 message handling centralized and deterministic.
- Preserve Unidirectional Data Flow (no view code mutating state directly).
- Improve testability of paint routing decisions.

## Non-Goals

- Redesigning the styling system or changing style definitions.
- Adding a new rendering backend.
- Changing app-level view models.

## Proposed Design

### 1) Centralize paint routing

Create a single routing function that maps (ControlKind, msg) to a paint
handler, instead of branching separately in each handler.

Suggested location: `src/CommanDuctUI/src/window_common.rs` or a new module
like `src/CommanDuctUI/src/controls/paint_router.rs`.

Example shape:

```rust
enum PaintRoute {
    LabelStatic,
    Edit,
    TreeViewCustomDraw,
    Default,
}

fn resolve_paint_route(kind: ControlKind, msg: u32) -> PaintRoute
```

Key rule:
- If ControlKind::Edit, route to edit styling even when msg is WM_CTLCOLORSTATIC.

### 2) Handler APIs accept ControlKind

Instead of calling handlers directly based on msg, call through the router:

- WM_CTLCOLORSTATIC -> resolve route -> label or edit handler
- WM_CTLCOLOREDIT -> resolve route -> edit handler (still supported)
- NM_CUSTOMDRAW -> route for TreeView (unchanged)

This keeps Win32-specific message wiring in one place and makes the routing
logic testable.

### 3) Make edit painting explicitly opaque

Ensure edit styling always sets:

- `SetBkMode(hdc, OPAQUE)`
- `SetBkColor` using style background (if provided)

This avoids stale pixels when scrolling.

### 4) Track control kind for read-only EDIT

Continue using existing `ControlKind::Edit` registration in
`execute_create_input` so routing decisions are reliable.

### 5) Logging & diagnostics

Add INFO-level log hooks in the router for unexpected combinations, e.g.:

- ControlKind::Edit + WM_CTLCOLORSTATIC (routed to edit)
- ControlKind::Static + WM_CTLCOLOREDIT (unexpected, still log)

Keep category prefix like `[Paint]` to filter easily.

## Implementation Steps

1) Add paint router module and unit tests for routing decisions.
2) Update WM_CTLCOLORSTATIC / WM_CTLCOLOREDIT handling in `window_common.rs`
   to call the router and then dispatch to the correct handler.
3) Ensure input_handler sets OPAQUE background (already done once).
4) Add logging to router (INFO).
5) Remove any legacy ad-hoc routing logic to prevent divergence.

## Unit Tests

Add tests for:

- `ControlKind::Edit + WM_CTLCOLORSTATIC => Edit route`
- `ControlKind::Edit + WM_CTLCOLOREDIT => Edit route`
- `ControlKind::Static + WM_CTLCOLORSTATIC => Label route`
- `ControlKind::Static + WM_CTLCOLOREDIT => Default/Ignore + log`

These should be pure tests (no Win32 calls).

## Risks / Edge Cases

- Some controls may be mis-registered with the wrong ControlKind.
  Add asserts in control creation to confirm the expected kind.
- If the router is placed in window_common, ensure no cycles with controls
  modules. Consider a small `controls::paint_router` module to avoid it.

## Migration Notes

- Start by introducing the router but keep existing behavior. Switch call
  sites one by one to minimize risk.
- Keep logs at INFO temporarily, then downgrade to DEBUG after validation.

## Success Criteria

- Scrolling preview (read-only EDIT) does not leave duplicated lines.
- Paint routing tests pass.
- Logs show routing decisions for read-only EDIT on WM_CTLCOLORSTATIC.
