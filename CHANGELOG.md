# Changelog

## 0.2.5 - 2026-02-10
- Enable best-effort dark theming for native Win32 menus by applying UXTheme dark-mode policy hooks.
- Re-apply dark-mode settings and redraw the menu bar immediately after `SetMenu` so `File` and popup menus render consistently.

## 0.2.4 - 2026-02-10
- Improve main event loop command responsiveness in idle periods

## 0.2.3 - 2026-01-27
- Add `TreeItemMarkerKind` palette hooks so the TreeView can request colored markers from the logic layer.
- Re-enable the custom-draw marker rendering path and paint a white ring plus a colored dot per item.
