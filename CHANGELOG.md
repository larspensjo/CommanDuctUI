# Changelog

## 1.0.6
- **Fix**: Owner-drawn list box selection now defaults to the warm visual-design palette instead of the older cool blue-gray fallback, so selected rows and the left accent bar match the app accent system out of the box.
- **Feature**: Add `StyleId::ListBoxSelectionAccent` and let list boxes consume `ListBoxRow` / `ListBoxSelectedRow` / `ListBoxSelectionAccent` / `ListBoxHoverRow` / `ListBoxDisabledRow` directly, so host apps can theme row states without patching `CommanDuctUI`.

## 1.0.5
- **Feature**: Add listbox keydown forwarding so host apps can handle control-local shortcuts without routing through the generic window message path.
- **Feature**: Extend owner-drawn listbox rows with additional badge and marker presentation hooks so hosts can attach richer row metadata without forking the control.
- **Fix**: Restore the generic `SecondaryButton` contract, including correct enabled-state updates for demoted non-primary footer actions.

## 1.0.4
- **Tests**: Rewrite brittle `treeview_handler` unit tests around invariant-level assertions instead of exact pixel rectangles, packed mask literals, and exact palette values where those details are implementation choices rather than the public behavior contract.
- **Hardening**: Keep explicit coverage for TreeView marker placement, reserved state-icon lane spacing, hidden-lane semantics, and warm/neutral marker palette intent while making harmless layout-tuning changes less likely to break the suite.

## 1.0.3
- **Fix**: Owner-drawn toggle switches now claim focus on click, so keyboard follow-up actions stay on the control the user just activated instead of remaining on the previous focus target.
- **Feature**: Owner-drawn tab bars are now keyboard-accessible. They claim focus on click, request arrow-navigation dialog keys, render a focus cue, and support `Left` / `Right` / `Home` / `End` selection changes in addition to mouse clicks.

## 1.0.2
- **Fix**: Owner-drawn list box now claims keyboard focus on click and returns the appropriate `WM_GETDLGCODE` flags so Arrow/Page/Home/End navigation reliably reaches the control inside dialog-style window message loops without over-claiming character input.
- **Hardening**: Add a shared `keyboard_navigation` helper for custom controls that centralizes the Win32 contract for dialog-navigation behavior (`WS_TABSTOP`, focus-on-click, and dialog-code negotiation) instead of re-encoding that behavior in each control WndProc.

## 1.0.1

- **Fix**: Badge text in the owner-drawn list box was drawn with the DC's default system font instead of `meta_font` because `SelectObject` was called to measure text width and then immediately restored before `DrawTextW` ran. Short labels such as "OK" were silently truncated to "O." at normal DPI. The font is now kept selected across both the measurement and the draw call using the new `SelectedObject` RAII guard.
- **Fix**: Badge column was always at least 130 px wide regardless of badge content, pushing list row titles far from their badges. The minimum is now 44 px so short status badges ("OK", "ERR") leave a tight, readable gap before the title.
- **Fix**: Owner-drawn list box scrollbar rendered in the system light theme. `try_enable_dark_mode` is now called on the list box window after creation so the native scrollbar inherits the app dark-mode policy.
- **Hardening**: Add `SelectedObject` RAII guard in `gdi_utils` that selects a GDI object into an HDC and restores the previous selection on `Drop`. Eliminates the class of bugs where a manual `SelectObject` restore is omitted on an early return or during future edits. Applied across `listbox_handler`, `chart_handler` (which had four separate early-return restore sites), `tab_bar_handler`, and `toggle_switch_handler`.

## 1.0.0

- Add an owner-drawn `ListBox` control contract for structured multi-line rows with badge descriptors, selection events, and programmatic selection commands
- Add `ListBoxRow` / `ListBoxSelectedRow` / `ListBoxHoverRow` / `ListBoxDisabledRow` styles plus badge styles for priority, category, status, and indirect markers
- Keep TreeView support intact for existing consumers while exposing the new list control as a separate platform primitive

## 0.10.7

- Add `StyleId::SecondaryButton` so host applications can demote non-primary actions without weakening every generic button surface
- Mute disabled button fills toward the warm neutral background instead of keeping full-strength active fills, so disabled destructive buttons no longer read as urgent active errors

## 0.10.6

- Keep a blank TreeView state-icon lane for `CheckState::Hidden` rows and erase the checkbox glyph in postpaint, so custom dots can sit between the expand button and the text instead of collapsing back onto the tree affordance

## 0.10.5

- Add `CheckState::Hidden` so TreeView rows can reserve the state-image lane without showing a checkbox, which lets host applications align custom markers cleanly while keeping interactive review rows opt-in
- Size TreeView markers from the text lane height so priority and status dots read as deliberate UI elements instead of tiny overlay pixels

## 0.10.4

- TreeView markers now anchor to the text lane instead of a fixed offset, which keeps the dot placement stable across checkbox and selection states
- Warmed the TreeView marker palette so shared download and priority markers better fit the dark neutral system

## 0.10.3

- Add `StyleId::StatusMeter`, `StyleId::SectionTitle`, `StyleId::PrimaryButton`, and `StyleId::DestructiveButton`
- Update chart and dialog hardcoded dark-theme colors to the warm neutral palette used by host applications

## 0.10.2

- Tests: rewrite generic infrastructure tests to use neutral menu fixtures and invariant-level assertions for layout validation, control description, and default tab-bar dark-theme behavior

## 0.10.1

- `chart_handler`: endpoint labels with deterministic overlap resolution (`place_end_labels`)
- `chart_handler`: `ChartLineEmphasis::Secondary` lines rendered with muted color and 1 px pen
- `chart_handler`: single-point series rendered as dot instead of skipped
- `chart_handler`: legend suppressed when `show_end_labels` is true

## 0.10.0

- `ChartLineData`: added `end_label: Option<String>` and `emphasis: ChartLineEmphasis`
- `ChartDataPacket`: added `show_x_axis_labels`, `show_y_axis_labels`, `show_end_labels` flags
- `chart_handler`: renders x-axis week labels and y-axis tick values when flags are set
- Left margin is now derived from measured y-axis label width instead of hard-coded value

## 0.9.1 - 2026-03-25
- **Fix**: Restore the `TreeViewSelectionAccent` bar for selected rows when `TreeViewSelectedRow` styling is active. Custom draw suppresses the native selected state during pre-paint, so post-paint now resolves the selected row from the TreeView caret item before drawing the accent.
- **Fix**: Route TreeView selection updates through `TVN_SELCHANGEDW` for user mouse/keyboard actions. This restores `TreeViewItemSelectionChanged` events for arrow-key navigation and keeps mouse and keyboard selection behavior aligned.

## 0.9.0 - 2026-03-24
- **Feature**: Add `AppEvent::WindowResizeCompleted { window_id, outer_width, outer_height }`. Emitted from the `WM_EXITSIZEMOVE` handler via `GetWindowRect`, giving subscribers the final outer window dimensions (frame + title bar included) after the user finishes a resize or move drag.

## 0.8.9 - 2026-03-22
- **Feature**: Add a reusable modal form dialog primitive with read-only rows, text inputs, checkboxes, and generic completion results. The primitive supports dark-theme-safe rendering plus live text-field validation and file-exists warnings for hosted applications.

## 0.8.8 - 2026-03-11
- **Fix**: Refine live resize and splitter-drag behavior for `TreeView`-heavy windows. The final interaction keeps background erase suppression during drag while leaving `TreeView` redraw enabled, preserving correct live updates without the blank-pane artifact from the abandoned freeze-based mitigation.

## 0.8.2 - 2026-03-10
- **Fix**: `show_window` now calls `RedrawWindow(RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW)` immediately after `ShowWindow(SW_SHOW)`, forcing a synchronous repaint of the window and all child controls. This eliminates the white-background flash visible between `ShowWindow` and the first message-loop paint cycle.
- **Fix**: `handle_set_rich_edit_content_command` re-applies the control's configured background and foreground colors (via `EM_SETBKGNDCOLOR` / `EM_SETCHARFORMAT`) after `EM_STREAMIN`, because streaming RTF content resets the background color previously set by `EM_SETBKGNDCOLOR`.

## 0.8.0 - 2026-03-10
- **BREAKING**: Add `TreeViewSelectedRow` and `TreeViewSelectionAccent` variants to `StyleId`.
- **Feature**: TreeView custom draw extended with opt-in selection styling. If `TreeViewSelectedRow` is defined, selected items render with a theme-matching background (suppressing the native blue highlight) and a full-width row fill. If `TreeViewSelectionAccent` is defined, a 3 px accent bar is drawn at x=0 of the panel for the selected row.
- **Fix**: `CDRF_NEWFONT` is now returned whenever any color (`clrText`/`clrTextBk`) is modified, not only when a custom font handle is present. Without this, Windows silently ignored color changes.
- Add `resolve_item_colors` pure helper (unit-testable, no Win32 dependency) for color precedence logic (base → per-item override → selection).

## 0.7.2 - 2026-03-03
- **Hardening**: `DefineLayout` validation now rejects docked edge rules (`Top/Bottom/Left/Right`) that omit `fixed_size` and rejects negative `fixed_size` values.
- **Hardening**: Add DPI-aware checkbox minimum-height helpers and enforce minimum native checkbox height in layout application path.
- **Tests**: Add layout regression coverage for header + checkbox + fill non-overlap and new validation failure cases.

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
