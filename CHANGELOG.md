# Changelog

## 0.7.1 - 2026-02-26
- **Bug fix**: `tab_bar_handler` now sends `WM_APP_TAB_SELECTED` to the root-ancestor window (`GetAncestor(hwnd, GA_ROOT)`) instead of the direct parent.
  Previously, clicks on tab bars nested inside panels were silently dropped because the panel WndProc called `DefWindowProcW` for unrecognised messages.

## 0.7.0 - 2026-06-11
- **BREAKING**: Add `TabBarSelectionChanged` variant to `AppEvent` enum.
- **Add** `PlatformCommand::CreateTabBar`, `SetTabBarItems`, `SetTabBarSelection`, `SetTabBarStyle`.
- Add `ControlKind::TabBar` and `StyleId::TabBar` / `StyleId::TabBarAccent`.
- Add `tab_bar_handler`: custom `HarvesterTabBarControl` Win32 window class with GDI paint.
  Renders tab labels, hover fill, 3 px accent underline for the active tab, and inactive text blend.
  Routes `WM_LBUTTONDOWN` click via `WM_APP_TAB_SELECTED` parent notification → `AppEvent::TabBarSelectionChanged`.
- Add `WM_APP_TAB_SELECTED` (`WM_APP + 0x104`) constant.
- All palette colors derived from background / text / accent with 40 % blend for inactive text and 6 % white overlay for hover fill.

## 0.6.0 - 2026-02-25
- Add `ChartDataPacket` and `ChartLineData` public types.
- **Add** `PlatformCommand::SetChartData { window_id, control_id, data }`.
- `chart_handler`: store chart data in GWLP_USERDATA (`ChartWindowState`);
  free Box on `WM_DESTROY`; paint dynamic multi-line chart with colored text legend;
  handle `SetChartData` command.

## 0.5.0 - 2026-02-25
- Add `ControlKind::Chart` (internal, pub(crate)).
- **Add** `PlatformCommand::CreateChart { window_id, parent_control_id, control_id }`.
- Add `chart_handler`: custom `HarvesterChartControl` window class with dark-theme GDI paint.
  Background `#1E2228`, dashed gridlines `#3A3F47`, hardcoded polyline `#4EC9B0`.
  WndProc handles `WM_ERASEBKGND` (no flicker), `WM_PAINT`, `WM_SIZE` (repaint on resize).

## 0.4.1 - 2026-02-25
- Validate `DefineLayout` input and reject layouts with multiple sibling `DockStyle::Fill` rules under the same parent (hard error instead of warning + silent degradation).
- Add unit tests covering layout validation for invalid duplicate Fill siblings and valid one-Fill-per-parent layouts.

## 0.4.0 - 2026-02-17
- **BREAKING**: Add `SetRadioButtonChecked` variant to `PlatformCommand` for explicit radio checked-state control from app state.
- Route `WM_CTLCOLORSTATIC` for `ControlKind::RadioButton` through button color handling so dark palette is applied on all radio paint paths.
- Disable themed rendering for radio buttons during style application (`SetWindowTheme("", "")`) so `WM_CTLCOLOR*` colors are respected consistently.

## 0.3.0 - 2026-02-16
- **BREAKING**: Add `ComboBox` and `RadioButton` variants to `ControlKind` enum.
- **BREAKING**: Add `ComboBox` and `RadioButton` variants to `StyleId` enum.
- **BREAKING**: Add `ComboBoxSelectionChanged` and `RadioButtonSelected` variants to `AppEvent` enum.
- Add `CreateComboBox`, `SetComboBoxItems`, `SetComboBoxSelection`, and `CreateRadioButton` commands to `PlatformCommand` enum.
- Add native ComboBox control support with dropdown list style (CBS_DROPDOWNLIST).
- Add native RadioButton control support with group start semantics (BS_AUTORADIOBUTTON with WS_GROUP).
- Add `WM_CTLCOLORLISTBOX` handling for ComboBox dropdown list dark theme support.
- Route `CBN_SELCHANGE` notifications to `ComboBoxSelectionChanged` events.
- Disambiguate `BN_CLICKED` notifications between push buttons and radio buttons using `ControlKind`.
- Add comprehensive unit tests for ComboBox and RadioButton handlers.

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
