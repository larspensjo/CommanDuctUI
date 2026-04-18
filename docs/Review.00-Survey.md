# Review 00 — Survey & framing

Phase 0 of [Plan.ThoroughReview](Plan.ThoroughReview.md). This document is the
reference map every later phase cites. It contains **no findings** — only the
structural facts needed to anchor subsequent analysis.

- **Crate:** `commanductui` v1.0.8, edition 2024
- **License:** `MIT OR Apache-2.0` (declared in `Cargo.toml`; only a single
  `LICENSE` file present on disk)
- **Total Rust source:** 18 307 lines across 32 files
- **Tests:** 130 inline `#[cfg(test)]` tests across 18 modules, all passing
- **Integration tests:** none (no `tests/` directory)

## Public API surface

The entire public surface is the re-export block in
[lib.rs](../src/lib.rs) (lines 32-41):

| Kind | Items |
|---|---|
| Entry point | `PlatformInterface` |
| Error | `PlatformResult` |
| Styling | `Color`, `ControlStyle`, `FontDescription`, `FontWeight`, `StyleId` |
| Types (traits) | `PlatformEventHandler`, `UiStateProvider` |
| Types (windows) | `WindowConfig`, `WindowId` |
| Types (controls) | `TreeItemDescriptor`, `TreeItemId`, `ListBoxItemDescriptor`, `ListBoxItemId`, `CheckState` |
| Types (dialogs/forms) | `FormDialogDescriptor`, `FormButtons`, `FormField`, `FormRow`, `FormFieldValue`, `FormFileExistsWarning`, `FormTextValidation` |
| Types (chart) | `ChartDataPacket`, `ChartLineData`, `ChartLineEmphasis`, `BadgeDescriptor` |
| Types (misc) | `AppEvent`, `PlatformCommand`, `MessageSeverity` |

Modules visible at `pub` scope: `app` (Windows-only), `error`, `types`.
Everything else is `pub(crate)`. On non-Windows targets only `error`, `types`,
and `styling_primitives` compile — `app`, `command_executor`, `controls`,
`window_common`, and `styling_windows` are gated behind `#[cfg(target_os = "windows")]`.

## File classification by responsibility

| Layer | File | LOC | Role |
|---|---|---:|---|
| Facade | [lib.rs](../src/lib.rs) | 41 | cfg-gated module wiring + re-exports |
| Facade | [controls.rs](../src/controls.rs) | 23 | `controls` submodule list |
| Platform-agnostic types | [types.rs](../src/types.rs) | 975 | `PlatformCommand`, `AppEvent`, descriptors, traits |
| Platform-agnostic styling | [styling_primitives.rs](../src/styling_primitives.rs) | 117 | `Color`, `ControlStyle`, `StyleId` |
| Error | [error.rs](../src/error.rs) | 60 | `PlatformError`, `PlatformResult` |
| Platform core | [app.rs](../src/app.rs) | 1681 | `PlatformInterface`, `Win32ApiInternalState`, top-level command dispatch |
| Platform core | [window_common.rs](../src/window_common.rs) | 3723 | window reg, WndProc, message loop, layout, paint, emits most `AppEvent`s |
| Platform core | [command_executor.rs](../src/command_executor.rs) | 837 | handler fns for "generic" commands |
| Platform styling | [styling_windows.rs](../src/styling_windows.rs) | 57 | `ParsedControlStyle` (Win32 resource-ready) |
| Platform styling | [styling_stub.rs](../src/styling_stub.rs) | 7 | non-Windows no-op shim |
| Dialogs | [controls/dialog_handler.rs](../src/controls/dialog_handler.rs) | 1907 | all 8 dialog flavors + form dialog |
| Tree controls | [controls/treeview_handler.rs](../src/controls/treeview_handler.rs) | 2022 | TreeView create/populate/custom-draw/events |
| List controls | [controls/listbox_handler.rs](../src/controls/listbox_handler.rs) | 1172 | custom owner-drawn list control |
| Primitive controls | [button_handler.rs](../src/controls/button_handler.rs) | 429 | button create + BN_CLICKED |
| Primitive controls | [checkbox_handler.rs](../src/controls/checkbox_handler.rs) | 267 | checkbox |
| Primitive controls | [combobox_handler.rs](../src/controls/combobox_handler.rs) | 520 | combobox + CBN_SELCHANGE |
| Primitive controls | [radiobutton_handler.rs](../src/controls/radiobutton_handler.rs) | 233 | radio groups |
| Primitive controls | [input_handler.rs](../src/controls/input_handler.rs) | 73 | edit-control styling |
| Primitive controls | [label_handler.rs](../src/controls/label_handler.rs) | 334 | static labels + status bar |
| Primitive controls | [panel_handler.rs](../src/controls/panel_handler.rs) | 218 | container panels |
| Primitive controls | [progress_handler.rs](../src/controls/progress_handler.rs) | 161 | progress bar |
| Primitive controls | [richedit_handler.rs](../src/controls/richedit_handler.rs) | 236 | RichEdit text content |
| Custom-drawn | [tab_bar_handler.rs](../src/controls/tab_bar_handler.rs) | 880 | custom tab bar |
| Custom-drawn | [toggle_switch_handler.rs](../src/controls/toggle_switch_handler.rs) | 522 | sliding toggle |
| Custom-drawn | [chart_handler.rs](../src/controls/chart_handler.rs) | 757 | GDI line chart |
| Custom-drawn | [splitter_handler.rs](../src/controls/splitter_handler.rs) | 372 | draggable splitter |
| Menu | [menu_handler.rs](../src/controls/menu_handler.rs) | 262 | main menu + WM_COMMAND routing |
| Cross-control util | [gdi_utils.rs](../src/controls/gdi_utils.rs) | 40 | RAII `SelectedObject` |
| Cross-control util | [keyboard_navigation.rs](../src/controls/keyboard_navigation.rs) | 101 | arrow-key helpers (shared by list/tab/toggle) |
| Cross-control util | [paint_router.rs](../src/controls/paint_router.rs) | 151 | paint dispatch by `ControlKind` |
| Cross-control util | [dark_border.rs](../src/controls/dark_border.rs) | 99 | dark-mode border overlay |
| Cross-control util | [styling_handler.rs](../src/controls/styling_handler.rs) | 30 | `Color ↔ COLORREF` conversion |

## Module dependency diagram

```
                         ┌──────────────────┐
                         │  types.rs        │◀───────── styling_primitives.rs
                         │  (cmds, events,  │
                         │   descriptors)   │
                         └──────────────────┘
                                  ▲
                                  │
                    ┌─────────────┴─────────────────────┐
                    │                                   │
        ┌──────────────────────┐              ┌───────────────────┐
        │       app.rs         │◀────────────▶│  window_common.rs │
        │  (PlatformInterface, │              │  (WndProc, loop,  │
        │   dispatch match)    │              │   layout, paint)  │
        └──────────────────────┘              └───────────────────┘
                 │  │                                   ▲
                 │  └────────────┐                      │
                 ▼               ▼                      │
     ┌──────────────────┐  ┌──────────────────┐         │
     │ command_executor │  │  controls/*      │─────────┘
     │ (generic cmds)   │─▶│  (per-control)   │
     └──────────────────┘  └──────────────────┘
                                   │
                                   ├─▶ controls::styling_handler (Color→COLORREF)
                                   ├─▶ controls::gdi_utils (RAII GDI)
                                   ├─▶ controls::keyboard_navigation (shared keys)
                                   └─▶ controls::paint_router (paint dispatch)
```

Edge observations (from `use crate::…` / `use super::…` imports):

- Every `controls/*` handler imports `app::Win32ApiInternalState`,
  `error::{PlatformError, PlatformResult}`, `types::*`, and
  `window_common::ControlKind`. `Win32ApiInternalState` is the universal seam.
- `command_executor` delegates to `listbox_handler`, `richedit_handler`, and
  `treeview_handler` — otherwise dispatch goes through `app.rs`.
- `window_common` imports control handlers via `super::{...}` (notifications
  originate here and are routed into handler functions).
- `controls::styling_handler`, `gdi_utils`, `keyboard_navigation`,
  `paint_router` are pure leaves — no reverse deps.
- No module imports the `styling_stub` directly; `lib.rs` aliases one of
  `styling_windows` / `styling_stub` as `styling` per target.

## `PlatformCommand` variants — grouped by theme

Source: [types.rs:537-880](../src/types.rs#L537). Dispatch: [app.rs:414-919](../src/app.rs#L414).

| Theme | Variants | Primary handler |
|---|---|---|
| Window lifecycle | `SetWindowTitle`, `ShowWindow`, `CloseWindow`, `QuitApplication`, `SignalMainWindowUISetupComplete`, `DefineLayout` | `command_executor` |
| Dialogs | `ShowSaveFileDialog`, `ShowOpenFileDialog`, `ShowProfileSelectionDialog`, `ShowInputDialog`, `ShowExcludePatternsDialog`, `ShowFormDialog`, `ShowMessageBox`, `ShowFolderPickerDialog` | `dialog_handler` |
| Menu | `CreateMainMenu` | `menu_handler` |
| Generic control state | `SetControlEnabled`, `SetControlText`, `SetScrollPosition` | `command_executor` |
| Styling | `DefineStyle`, `ApplyStyleToControl` | `app.rs` (inline methods) |
| Tree | `CreateTreeView`, `PopulateTreeView`, `UpdateTreeItemVisualState`, `UpdateTreeItemText`, `RedrawTreeItem`, `ExpandVisibleTreeItems`, `ExpandAllTreeItems`, `SetTreeViewSelection` | `treeview_handler` / `command_executor` |
| List | `CreateListBox`, `PopulateListBox`, `SetListBoxSelection` | `command_executor` / `listbox_handler` |
| Button | `CreateButton` | `button_handler` |
| Panel | `CreatePanel` | `panel_handler` |
| Label | `CreateLabel`, `UpdateLabelText` | `label_handler` |
| Edit/Viewer | `CreateInput`, `SetInputText`, `SetViewerContent` | `command_executor` |
| RichEdit | `CreateRichEdit`, `SetRichEditContent` | `command_executor` / `richedit_handler` |
| Chart | `CreateChart`, `SetChartData` | `chart_handler` |
| ProgressBar | `CreateProgressBar`, `SetProgressBarRange`, `SetProgressBarPosition` | `progress_handler` |
| Splitter | `CreateSplitter` | `splitter_handler` |
| ComboBox | `CreateComboBox`, `SetComboBoxItems`, `SetComboBoxSelection` | `combobox_handler` |
| RadioButton | `CreateRadioButton`, `SetRadioButtonChecked` | `radiobutton_handler` |
| CheckBox | `CreateCheckBox`, `SetCheckBoxChecked` | `checkbox_handler` |
| TabBar | `CreateTabBar`, `SetTabBarItems`, `SetTabBarSelection`, `SetTabBarStyle` | `tab_bar_handler` |
| ToggleSwitch | `CreateToggleSwitch`, `SetToggleSwitchState`, `SetToggleSwitchStyle` | `toggle_switch_handler` |

**Variant count:** 51 `PlatformCommand` variants total.

## `AppEvent` variants — grouped by theme

Source: [types.rs:229-383](../src/types.rs#L229).

| Theme | Variants | Emission site(s) |
|---|---|---|
| Window lifecycle | `WindowCloseRequestedByUser`, `WindowResized`, `WindowResizeCompleted`, `WindowDestroyed`, `MainWindowUISetupComplete` | `window_common.rs` |
| Tree | `TreeViewItemToggledByUser`, `TreeViewItemSelectionChanged` | `treeview_handler.rs` |
| List | `ListBoxItemSelectionChanged`, `ListBoxItemKeyDown`, `ListBoxScrolled` | `window_common.rs` |
| Button | `ButtonClicked` | `button_handler.rs` |
| Menu | `MenuActionClicked` | `menu_handler.rs` |
| Dialog completions | `FileSaveDialogCompleted`, `FileOpenProfileDialogCompleted`, `ProfileSelectionDialogCompleted`, `GenericInputDialogCompleted`, `FormDialogCompleted`, `ExcludePatternsDialogCompleted`, `FolderPickerDialogCompleted` | `dialog_handler.rs` |
| Scroll/input | `ControlScrolled`, `InputTextChanged` | `window_common.rs` |
| Splitter | `SplitterDragging`, `SplitterDragEnded` | `window_common.rs` |
| ComboBox | `ComboBoxSelectionChanged` | `combobox_handler.rs` |
| RadioButton | `RadioButtonSelected` | `window_common.rs` |
| CheckBox | `CheckBoxToggled` | `window_common.rs` |
| TabBar | `TabBarSelectionChanged` | `window_common.rs` |
| ToggleSwitch | `ToggleSwitchToggled` | `window_common.rs` |

**Variant count:** 27 `AppEvent` variants total.

**Observation for later phases:** a clear pattern emerges — events for
*standard Win32 notifications* (WM_COMMAND → BN_CLICKED, CBN_SELCHANGE) mostly
live in the per-control handler, but events for *custom/owner-drawn controls*
(tab bar, toggle, splitter) and for window-global concerns (resize, scroll,
radio/check/list) are emitted from `window_common.rs`. Whether this split is
desired or accidental is a Phase 1 architecture question.

## Tests

`cargo test --lib` → **130 passed; 0 failed; 0 ignored** (0.01 s).

| Module | `#[test]` count |
|---|---:|
| `window_common::tests` | 27 |
| `controls::treeview_handler` | 20 |
| `controls::chart_handler` | 14 |
| `app::tests` | 12 |
| `command_executor::tests` | 8 |
| `controls::listbox_handler` | 7 |
| `controls::tab_bar_handler` | 6 |
| `controls::paint_router` | 10 |
| `controls::combobox_handler` | 4 |
| `controls::checkbox_handler` | 4 |
| `controls::keyboard_navigation` | 4 |
| `controls::button_handler` | 3 |
| `controls::radiobutton_handler` | 3 |
| `controls::menu_handler` | 2 |
| `controls::panel_handler` | 2 |
| `types::tests` | 2 |
| `controls::dialog_handler` | 1 |
| `controls::label_handler` | 1 |

**Modules with zero `#[test]`:** `styling_primitives`, `styling_windows`,
`styling_stub`, `error`, `splitter_handler`, `progress_handler`,
`input_handler`, `richedit_handler`, `toggle_switch_handler`, `gdi_utils`,
`dark_border`, `styling_handler`.

No doctests (no `cargo test --doc` output yet; public items are not
example-carrying — see Phase 2).

## Release-readiness at a glance

(Anchors only — full audit lives in Phase 2.)

- `Cargo.toml` has: `name`, `version`, `edition`, `license`, `description`.
- `Cargo.toml` is **missing**: `keywords`, `categories`, `repository`,
  `documentation`, `readme`, `authors` (optional in edition 2024 but common),
  `rust-version`.
- `LICENSE` file exists (single file). `Cargo.toml` declares `MIT OR
  Apache-2.0` dual license — Phase 2 will verify whether the single
  `LICENSE` file satisfies that declaration or whether `LICENSE-MIT` and
  `LICENSE-APACHE` are required.
- `CHANGELOG.md` present (15 KB).
- `README.md` present (6.4 KB).
- No `examples/` directory.
- No CI workflow files detected in the repository root (`.github/` not
  enumerated here — Phase 2 will confirm).

## Notable TODOs in source

Surface-scan only; each is a candidate finding for its relevant phase:

- `types.rs:175` — "TODO: I think many of these have not been implemented
  yet" on `DockStyle` variants.
- `types.rs:387` — "TODO: 'None' isn't used, is it needed?" on
  `MessageSeverity::None`.
- `types.rs:398` — "TODO: Only 'StatusBar' is currently used, is it needed?"
  on `LabelClass`.
- `types.rs:534` — "TODO: All commands that create controls should use the
  same name for this ID. E.g. 'control_id'."
- `types.rs:672` — "TODO: Now that 'CreateInput' can be used to create a
  read-only item, it should probably change name."
- `error.rs:8-9` — "TODO: Where are these taken care of?" / "TODO: Usually,
  these are created at the same time as a log::error!, etc. Maybe
  unnecessary duplication?"

## Cross-cutting seams that later phases should probe

1. **`Win32ApiInternalState` is the universal context** — every control
   handler takes `&Arc<Win32ApiInternalState>`. Phase 1 should ask whether
   this is a god-object or a legitimate registry.
2. **`window_common.rs` is both the message-loop and a de-facto event-emitter
   for ~13 of the 27 `AppEvent` variants.** Phase 1 seam candidate.
3. **Styling is a two-layer split**: `styling_primitives` (portable) +
   `styling_windows`/`styling_stub` (platform). The aliased `use styling_x
   as styling` keeps handlers target-agnostic at the import level.
4. **`command_executor` vs. per-control handlers** — some commands dispatch
   to `command_executor`, others directly to `controls::*`. The split is
   historical, not architectural. Phase 1 should rationalise this.
5. **Shared mutable state** observable in `app.rs`:
   - `RwLock<HashMap<WindowId, NativeWindowData>>` (active windows)
   - `Mutex<Option<Weak<Mutex<dyn PlatformEventHandler>>>>`
   - `Mutex<Option<Weak<Mutex<dyn UiStateProvider>>>>`
   - `RwLock<HashMap<StyleId, Arc<ParsedControlStyle>>>`
   - `AtomicUsize` quit flag + window id counter
   - `unsafe impl Send/Sync for Win32ApiInternalState` — Phase 3 must verify.

---

No findings recorded in this phase. Findings begin with
[Review.01-Architecture.md](Review.01-Architecture.md) (Phase 1).
