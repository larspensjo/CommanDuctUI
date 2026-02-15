# Changelog

## 0.2.8 - 2026-02-15
- Force a full-window redraw pass after layout recalculation (`RedrawWindow` with `RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW`) to eliminate residual paint artifacts during dynamic relayout.

## 0.2.7 - 2026-02-13
- Add `ViewerReadable` variant to `StyleId` enum so apps can assign a prose-friendly preview style without changing existing monospace usages.
- Add `PlatformCommand::CreateRichEdit` and `PlatformCommand::SetRichEditContent`.
- Add Rich Edit control creation/registration and RTF content streaming via `EM_STREAMIN`.

## 0.2.6 - 2026-02-13
- Add `TreeItemDisabled` variant to `StyleId` enum for muted-gray styling of tree items that lack associated data.

## 0.2.5 - 2026-02-10
- Enable best-effort dark theming for native Win32 menus by applying UXTheme dark-mode policy hooks.
- Re-apply dark-mode settings and redraw the menu bar immediately after `SetMenu` so `File` and popup menus render consistently.

## 0.2.4 - 2026-02-10
- Improve main event loop command responsiveness in idle periods

## 0.2.3 - 2026-01-27
- Add `TreeItemMarkerKind` palette hooks so the TreeView can request colored markers from the logic layer.
- Re-enable the custom-draw marker rendering path and paint a white ring plus a colored dot per item.
