# Review 01 — Architecture & module boundaries

Phase 1 findings from [Plan.ThoroughReview](Plan.ThoroughReview.md). Each
finding is recorded once here and mirrored in
[Review.Backlog.md](Review.Backlog.md).

## End-to-end traces

### `CreateButton` command

Path from enqueue to native resource:

1. Host app enqueues `PlatformCommand::CreateButton { … }` (definition:
   [types.rs:627-633](../src/types.rs#L627)).
2. `PlatformInterface::main_event_loop` drains commands via
   `app_logic_ref_for_loop.lock().try_dequeue_command()` ([app.rs:1370](../src/app.rs#L1370)).
3. Each command enters `Win32ApiInternalState::execute_platform_command`
   ([app.rs:412](../src/app.rs#L412)) — a 500-line `match` over all 51
   variants.
4. `CreateButton` dispatches to
   `button_handler::handle_create_button_command` ([app.rs:565](../src/app.rs#L565)).
5. Handler runs a three-phase create ([button_handler.rs:54-178](../src/controls/button_handler.rs#L54)):
   read-lock pre-check → `CreateWindowExW` without locks → write-lock
   register. Uses `with_window_data_{read,write}` helpers.

**Observation.** Dispatch is a single flat `match` in `app.rs`. Variants
are split across two destinations — `command_executor::execute_*` (for
"generic" commands) and per-control `handle_*_command` functions. The
split is historical, not principled (see F-01-005).

### `ButtonClicked` event

Path from native notification to host:

1. User clicks → `WM_COMMAND` arrives at the window's `WndProc`
   ([window_common.rs:1915](../src/window_common.rs#L1915)).
2. `handle_wm_command` dispatches on the `highord` notification code
   ([window_common.rs:2250](../src/window_common.rs#L2250)); for
   `BN_CLICKED` on a `ControlKind::Button` it calls
   `button_handler::handle_bn_clicked(window_id, control_id, hwnd)`
   ([window_common.rs:2307](../src/window_common.rs#L2307)).
3. `handle_bn_clicked` constructs `AppEvent::ButtonClicked { … }` and
   returns it ([button_handler.rs:183](../src/controls/button_handler.rs#L183)).
4. Back in the generic WndProc trailer, `self.send_event(event)` is called
   ([window_common.rs:2052](../src/window_common.rs#L2052)).
5. `send_event` upgrades the `Weak<Mutex<dyn PlatformEventHandler>>`,
   locks it, and invokes `handler_guard.handle_event(event)` **synchronously
   on the UI thread** ([app.rs:190-207](../src/app.rs#L190)).

**Observation.** Events are not queued. The host's `handle_event`
executes inside the WndProc call stack. This constrains what the host
can do there (no blocking I/O, no re-entrant dispatch). See F-01-002.

## Data-flow invariant (`input → action → reducer → state → render`)

The invariant declared in [Agents.md:11](../Agents.md#L11) concerns the
overall SourcePacker application. For CommanDuctUI's role the picture is:

```
 ┌─────────────┐   ┌──────────────────┐   ┌─────────────────────────────┐
 │ Win32 WM_*  │──▶│ window_common /  │──▶│  send_event (sync, on UI    │
 │ (input)     │   │ controls/*       │   │  thread) → host's           │
 └─────────────┘   │ (action = event) │   │  handle_event (reducer)     │
                   └──────────────────┘   └─────────────────────────────┘
                                                        │
                                                        ▼
                                               host state + command queue
                                                        │
                                                        ▼
                                   ┌───────────────────────────────────┐
                                   │ main_event_loop drains queue via  │
                                   │ try_dequeue_command → dispatch →  │
                                   │ execute_platform_command (render) │
                                   └───────────────────────────────────┘
```

**Verdict:** the library *enforces* the contract rather than violating
it. Events are platform-agnostic; the host cannot directly call Win32.
Commands are platform-agnostic; the host cannot directly touch HWNDs.
The library sits symmetrically on both sides of the reducer.

**Caveats that matter for later phases:**

1. The reducer runs on the UI thread. Any slow host handler blocks the
   message loop. The code includes `SLOW_MESSAGE_THRESHOLD_MS`
   profiling hooks precisely because this is a known sharp edge.
2. The command loop uses polling rather than wake-on-post — see
   F-01-001.
3. Nothing in the library is a reducer; there are no pure-function
   reducer seams to unit-test per [Agents.md:18](../Agents.md#L18). The
   library's testability burden is about effect-emission shape, not
   reducer behaviour. Phase 5 will revisit.

## Shared mutable state inventory

From `app.rs:60-72`:

| Field | Type | Access pattern |
|---|---|---|
| `h_instance` | `HINSTANCE` | immutable after construction |
| `next_window_id_counter` | `AtomicUsize` | `fetch_add(Relaxed)` |
| `active_windows` | `RwLock<HashMap<WindowId, NativeWindowData>>` | read for lookups, write for create/destroy, `with_window_data_{read,write}` helpers |
| `application_event_handler` | `Mutex<Option<Weak<Mutex<dyn PlatformEventHandler>>>>` | set once in `main_event_loop`, read per-event |
| `ui_state_provider` | `Mutex<Option<Weak<Mutex<dyn UiStateProvider>>>>` | set once, read during treeview custom-draw |
| `defined_styles` | `RwLock<HashMap<StyleId, Arc<ParsedControlStyle>>>` | write on `DefineStyle`, read on every paint |
| `app_name_for_class` | `String` | immutable after construction |
| `is_quitting` | `AtomicUsize` (0/1) | `store(Relaxed)` / `load(Relaxed)` |

Plus:

- `LOAD_RICHEDIT_DLL_ONCE: Once` at [app.rs:46](../src/app.rs#L46).
- One `thread_local! PROGRAMMATIC_SCROLL_SUPPRESSIONS: RefCell<HashSet<…>>`
  at [window_common.rs:132](../src/window_common.rs#L132) (tied to UI
  thread, consumed by `ProgrammaticScrollGuard`).
- `OnceLock`s for dark-mode function pointers, control-class registration
  tokens (`CHART_CLASS_REGISTERED`, `LIST_BOX_CLASS_REGISTERED`,
  `TAB_BAR_CLASS_REGISTERED`, `TOGGLE_SWITCH_CLASS_REGISTERED`), and a
  `OnceLock<Mutex<HashSet<isize>>> THEMED_HWNDS` at
  [window_common.rs:2857](../src/window_common.rs#L2857).
- `unsafe impl Send/Sync for Win32ApiInternalState` ([app.rs:75-76](../src/app.rs#L75))
  and `unsafe impl Send/Sync for ParsedControlStyle`
  ([styling_windows.rs:30-31](../src/styling_windows.rs#L30)). Phase 3
  will verify these.

**No `static mut`, no `lazy_static!`, no `RefCell` outside the
`thread_local!`.** This is good — the state is properly scoped.

**One `Mutex<Option<Weak<Mutex<…>>>>` quirk.** The outer `Mutex` is only
held for a lookup-and-upgrade operation; the inner `Mutex` is held
across the user's `handle_event` call. No nested holding, so no
deadlock; but the shape is unusual and worth a comment pin. See
F-01-011.

## Styling platform-split review

```
                 styling_primitives.rs (always built)
                      │
                      ├── Color, FontDescription, ControlStyle, StyleId
                      │
            ┌─────────┴──────────┐
            │                    │
styling_windows.rs      styling_stub.rs
  (target = windows)    (otherwise)
      │                        │
      ├── re-exports primitives    ├── re-exports primitives
      └── ParsedControlStyle       (no extra types)
           (HFONT + HBRUSH + Drop)

lib.rs: pub(crate) use styling_{windows|stub} as styling;
```

The split is **correct in shape**. `ParsedControlStyle` is Windows-only
by construction (it holds `HFONT`/`HBRUSH`) and is `pub(crate)` so it
never leaks to downstream crates.

Two snags:

1. [listbox_handler.rs:21](../src/controls/listbox_handler.rs#L21)
   imports `use crate::styling_windows::ParsedControlStyle;` directly,
   bypassing the `styling` alias that every other site uses. See F-01-010.
2. `ParsedControlStyle` derives `Clone` and implements `Drop` that
   releases GDI resources. Today nothing calls `.clone()` on it, so
   the soundness hazard is latent; a future edit could silently
   introduce a double-`DeleteObject`. See F-01-003 (Critical).

## Hot-file decomposition sketches

The plan (line 30) asks for seams, not refactors. Each sketch below
proposes a split the corresponding file can land in a follow-up PR
after separate approval.

### `window_common.rs` (3723 LOC) — F-01-006

Natural seams already visible as top-level groups:

| Proposed module | LOC est. | Contents |
|---|---:|---|
| `window_common/programmatic_scroll.rs` | ~60 | `thread_local!`, `ProgrammaticScrollGuard` |
| `window_common/native_window_data.rs` | ~1050 | `NativeWindowData` + all `pub(crate)` methods + `Drop` |
| `window_common/window_creation.rs` | ~200 | `register_window_class`, `create_native_window`, `destroy_native_window`, `show_window`, `send_close_message`, `set_window_title` |
| `window_common/wparam_utils.rs` | ~20 | `loword_from_wparam`/`hiword_from_lparam` |
| `window_common/dark_mode.rs` | ~400 | `init_app_dark_mode`, `try_enable_dark_mode`, `DarkModeUxThemeOrdinals`, `MenuBarColors`, `apply_button_dark_mode_classic_render` |
| `window_common/message_dispatch.rs` | ~1000 | the big `WndProc` + `handle_wm_command` + `handle_wm_notify_dispatch` |
| `window_common/mod.rs` | ~100 | re-exports + `Win32ApiInternalState::message_name`/`should_profile_message` + `read_edit_control_text` |
| `window_common/tests.rs` | ~200 | existing tests + wiring |

Reducing `window_common.rs` to ~100 lines of facade would simplify every
Phase 2+ audit.

### `treeview_handler.rs` (2022 LOC) — F-01-007

Top-level functions already cluster cleanly:

| Proposed module | Contents |
|---|---|
| `treeview/state.rs` | `TreeViewInternalState` and its impls (lines 80-214) |
| `treeview/commands.rs` | `handle_create_treeview_command`, `populate_treeview`, `update_*`, `expand_*`, `set_treeview_selection`, `handle_redraw_tree_item_command` |
| `treeview/selection.rs` | `handle_treeview_itemchanged_notification`, `handle_treeview_selection_changed_notification`, `is_user_treeview_selection_action` |
| `treeview/checkbox.rs` | `handle_wm_app_treeview_checkbox_clicked`, `handle_nm_click`, `treeview_state_image_mask` |
| `treeview/custom_draw.rs` | `handle_nm_customdraw` (~310 lines alone) + `resolve_item_colors`, `treeview_tail_fill_rect`, `draw_tree_item_marker`, etc. |
| `treeview/geometry.rs` | `rect_seeded_for_treeview_item`, `treeview_item_rect`, `tree_item_state_icon_lane_rect`, `tree_item_marker_rect`, `tree_item_marker_color` |
| `treeview/mod.rs` | re-exports + short facade |

### `dialog_handler.rs` (1907 LOC) — F-01-008

Current file is a flat sequence of ~8 independent dialogs. Proposed:

| Proposed module | Contents |
|---|---|
| `dialog/common.rs` | `pathbuf_from_buf`, `get_hwnd_owner`, `show_common_file_dialog` generic helper, `form_background_brush`, template byte helpers (`push_word`, `push_str_utf16`, `align_to_dword`) |
| `dialog/file_dialogs.rs` | `handle_show_save_file_dialog_command`, `handle_show_open_file_dialog_command` |
| `dialog/profile_selection.rs` | `build_profile_dialog_template`, `handle_show_profile_selection_dialog_command` |
| `dialog/form_dialog.rs` | form dialog template builder + `handle_show_form_dialog_command` + validation helpers (~800 lines — the largest single dialog) |
| `dialog/input_dialog.rs` | `build_input_dialog_template`, `handle_show_input_dialog_command` |
| `dialog/exclude_patterns.rs` | `build_exclude_patterns_dialog_template`, `handle_show_exclude_patterns_dialog_command` |
| `dialog/message_box.rs` | `message_box_icon_flag`, `handle_show_message_box_command` |
| `dialog/folder_picker.rs` | `handle_show_folder_picker_dialog_command` |

### `app.rs` (1681 LOC) — F-01-009

`Win32ApiInternalState::execute_platform_command` is ~500 lines of `match`
and `Win32ApiInternalState::define_style` is ~120 lines. Proposed:

| Proposed module | Contents |
|---|---|
| `app/interface.rs` | `PlatformInterface` + `Win32ApiInternalState::new`, helper accessors, `main_event_loop` |
| `app/command_dispatch.rs` | `execute_platform_command` alone |
| `app/styling.rs` (or merge into `styling_windows.rs`) | `define_style` and `execute_apply_style_to_control` (Win32 parsing belongs with `ParsedControlStyle`) |
| `app/send_event.rs` | `send_event` + the `Weak`-upgrade helper (also used for `UiStateProvider`) |

## Findings

### F-01-001: Event loop polls every 15 ms instead of waking on command
- **Severity:** Major
- **Dimension:** arch
- **Status:** open
- **Location:** [src/app.rs:1392-1422](../src/app.rs#L1392)
- **Observation:** `main_event_loop` uses `PeekMessageW(PM_REMOVE)` and, when no message is queued, `std::thread::sleep(15 ms)` before looping again. The stated reason is "sleep briefly so command dequeue stays responsive without user input."
- **Why it matters:** idle CPU wake-ups every 15 ms across every running app instance; up to 15 ms latency for a command enqueued from a background thread; the pattern is not the recommended Win32 event-loop shape. `MsgWaitForMultipleObjects` (wake on message **or** a manual-reset event signalled by a `try_enqueue_command` notifier) is the idiomatic fix.
- **Recommendation:** expose a `wake_event: HANDLE` on `Win32ApiInternalState` that the host can signal (or that a wrapper signals inside `try_enqueue_command`). Replace `PeekMessageW + sleep` with `MsgWaitForMultipleObjectsEx([wake_event], INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)` followed by a standard message pump. Preserve a busy-drain of the command queue at each wake.

### F-01-002: `send_event` runs host `handle_event` synchronously on the UI thread
- **Severity:** Minor
- **Dimension:** arch
- **Status:** fixed
- **Location:** [src/app.rs:190-207](../src/app.rs#L190)
- **Observation:** Events are delivered to the host's `PlatformEventHandler` inline from `WndProc`. There is no queue between native notification and host reducer.
- **Why it matters:** This is a legitimate design choice (simple, no per-event allocation, preserves message ordering), but it's a public contract that is currently undocumented on `PlatformEventHandler`. A host that performs blocking I/O in `handle_event` or re-enters the library (e.g. opens a modal dialog) can deadlock or starve input. This is the single most load-bearing invariant of the public API and must be called out.
- **Recommendation:** add a "Threading & re-entrancy" section to the `PlatformEventHandler` rustdoc: (1) `handle_event` runs on the UI thread inside `WndProc`; (2) handlers must not block; (3) enqueueing commands is safe; (4) triggering another event synchronously is re-entrant and unsupported. Phase 2 will fold this into the public-docs audit.

### F-01-003: `ParsedControlStyle` derives `Clone` while owning GDI resources via `Drop`
- **Severity:** Critical
- **Dimension:** correctness
- **Status:** fixed
- **Location:** [src/styling_windows.rs:18-56](../src/styling_windows.rs#L18)
- **Observation:** `#[derive(Clone)] pub(crate) struct ParsedControlStyle { font_handle: Option<HFONT>, …, background_brush: Option<HBRUSH> }` and `impl Drop for ParsedControlStyle { /* DeleteObject on both handles */ }`. No call sites currently invoke `.clone()` on it, but the derive silently permits double-free.
- **Why it matters:** a future edit — even a trivial "let me make a copy of this palette for a test" — would compile without warning and produce a double `DeleteObject`, which is undefined behaviour (MSDN marks `DeleteObject` on an already-freed handle as UB; at best Win32 returns `FALSE`, at worst the handle has been recycled and the new owner's resource is destroyed).
- **Recommendation:** remove `#[derive(Clone)]`. If Clone is truly needed, implement it manually by duplicating the underlying GDI objects (`CreateFontIndirectW` from a `LOGFONTW` read via `GetObjectW`, `CreateSolidBrush` from a re-extracted color). Today, nothing needs Clone — the struct is always accessed behind an `Arc<ParsedControlStyle>`.

### F-01-004: `AppEvent` emission is split unevenly between `window_common` and per-control handlers
- **Severity:** Major
- **Dimension:** arch
- **Status:** open
- **Location:** [src/window_common.rs](../src/window_common.rs) vs [src/controls/*_handler.rs](../src/controls)
- **Observation:** 13 of the 27 `AppEvent` variants are constructed directly in `window_common.rs` (see Phase 0 table), including `RadioButtonSelected`, `CheckBoxToggled`, `ListBoxItemSelectionChanged`, `TabBarSelectionChanged`, `ToggleSwitchToggled`, etc. — controls that have their own handler module. Only `BN_CLICKED` and `CBN_SELCHANGE` cases delegate to `button_handler`/`combobox_handler` for event translation; the other notifications are translated inline.
- **Why it matters:** "inline in the WndProc" becomes the default, which keeps `window_common.rs` growing. Each new custom-drawn control adds another emit site rather than another file. Phase 4 DRY audits and Phase 5 tests will both be harder to do cleanly under the current split.
- **Recommendation:** move every `AppEvent::*` construction into the corresponding `controls/*_handler.rs`. `handle_wm_command` / `handle_wm_notify_dispatch` / the `WM_APP_*` branches in `window_common.rs` should return an `Option<AppEvent>` from a thin call into the handler module, mirroring the existing `button_handler::handle_bn_clicked` pattern. This is a pure relocation — no behavioural change, easy to do incrementally, one control at a time.

### F-01-005: Command dispatch splits arbitrarily between `command_executor` and per-control handlers
- **Severity:** Minor
- **Dimension:** arch
- **Status:** open
- **Location:** [src/app.rs:412-919](../src/app.rs#L412)
- **Observation:** In the 500-line `execute_platform_command` match, some variants delegate to `command_executor::execute_*` (e.g. `CreateListBox`, `CreateInput`, `SetControlText`) and some to `controls::*::handle_*_command` (e.g. `CreateButton`, `CreateTreeView`, `CreateComboBox`). There is no pattern — `CreateListBox` lives in `command_executor` while `CreateComboBox` lives in `combobox_handler`.
- **Why it matters:** reviewers and new contributors cannot predict where a given command's implementation lives. The split is a historical artefact of the extraction from SourcePacker.
- **Recommendation:** move every command implementation into its control's handler module. Keep `command_executor` only for commands that are not tied to a specific control (`DefineLayout`, `QuitApplication`, `SignalMainWindowUISetupComplete`, generic `SetControlText`/`SetControlEnabled`/`SetScrollPosition`). Do this on the same schedule as F-01-004 — one control at a time, no behaviour change.

### F-01-006: `window_common.rs` at 3723 LOC hosts unrelated concerns
- **Severity:** Major
- **Dimension:** arch
- **Status:** open
- **Location:** [src/window_common.rs](../src/window_common.rs)
- **Observation:** the file contains `NativeWindowData` and its ~35 methods, dark-mode support, menu-bar color parsing, scroll suppression, window class registration, the WndProc dispatcher (~1000 LOC alone), `handle_wm_command`/`handle_wm_notify_dispatch`, layout recalculation, plus standalone helpers for window show/close/title. Seams are already visible as contiguous line ranges (see decomposition sketch above).
- **Why it matters:** every Phase 3/4 audit has to scan the whole file for cross-cutting concerns that are in reality well-separated. Tests already account for 27 `#[test]` items in this file; splitting does not require new tests, only moves.
- **Recommendation:** split into a `window_common/` subdirectory with seven modules as sketched above. Target: reduce `window_common.rs` to a ~100-line `mod.rs` facade.

### F-01-007: `treeview_handler.rs` at 2022 LOC mixes state, custom-draw, and selection/check logic
- **Severity:** Major
- **Dimension:** arch
- **Status:** open
- **Location:** [src/controls/treeview_handler.rs](../src/controls/treeview_handler.rs)
- **Observation:** `handle_nm_customdraw` alone is ~310 lines of paint logic that has nothing to do with `TreeViewInternalState` book-keeping. Selection, checkbox handling, command execution, and geometry helpers are interleaved.
- **Recommendation:** split into `controls/treeview/{state,commands,selection,checkbox,custom_draw,geometry,mod}.rs` as sketched.

### F-01-008: `dialog_handler.rs` at 1907 LOC bundles eight independent dialog flows
- **Severity:** Major
- **Dimension:** arch
- **Status:** open
- **Location:** [src/controls/dialog_handler.rs](../src/controls/dialog_handler.rs)
- **Observation:** the form dialog alone is ~800 lines; the other dialogs (file save/open, profile selection, input, exclude patterns, message box, folder picker) are each independent and share only a thin template-byte helper layer.
- **Recommendation:** split into `controls/dialog/{common,file_dialogs,profile_selection,form_dialog,input_dialog,exclude_patterns,message_box,folder_picker,mod}.rs` as sketched.

### F-01-009: `app.rs` mixes `PlatformInterface` lifecycle with command dispatch and style parsing
- **Severity:** Major
- **Dimension:** arch
- **Status:** open
- **Location:** [src/app.rs](../src/app.rs) lines 412-919 (dispatch) and 937-1055 (`define_style`)
- **Observation:** the 500-line `execute_platform_command` match and the 120-line `define_style` method do not belong next to `PlatformInterface::new` / `main_event_loop`.
- **Recommendation:** lift `execute_platform_command` into `app/command_dispatch.rs` (or, once F-01-005 is applied, split per-control). Lift `define_style` + `execute_apply_style_to_control` into `styling_windows.rs` next to `ParsedControlStyle` — the logic is about *constructing* parsed styles and belongs with the type.

### F-01-010: `listbox_handler` imports `ParsedControlStyle` via `styling_windows` instead of the `styling` alias
- **Severity:** Minor
- **Dimension:** arch
- **Status:** fixed
- **Location:** [src/controls/listbox_handler.rs:20-21](../src/controls/listbox_handler.rs#L20)
- **Observation:** lists two imports: `use crate::styling_primitives::StyleId;` and `use crate::styling_windows::ParsedControlStyle;`. Every other handler imports from the target-aliased `crate::styling` module (which already re-exports `StyleId`).
- **Why it matters:** bypasses the cfg-alias indirection that the module structure depends on. If the `styling_stub` path ever needs to expose a `ParsedControlStyle` shim (e.g. for a cross-platform test target), this import would not follow.
- **Recommendation:** import via `use crate::styling::{StyleId, ParsedControlStyle};` — requires re-exporting `ParsedControlStyle` from `styling_windows`, which is already `pub(crate)`. Same for the `crate::styling_primitives::StyleId` line.

### F-01-011: `Mutex<Option<Weak<Mutex<dyn …>>>>` on handler/provider slots is unusual
- **Severity:** Nit
- **Dimension:** arch
- **Status:** fixed
- **Location:** [src/app.rs:65-67](../src/app.rs#L65)
- **Observation:** `application_event_handler` and `ui_state_provider` are typed `Mutex<Option<Weak<Mutex<dyn Trait>>>>`. The outer `Mutex` guards a single "set once at startup" assignment; the `Weak` allows the host to own the strong reference and drop without leaking; the inner `Mutex` locks per-call.
- **Why it matters:** the shape is load-bearing (must support the host's `Arc<Mutex<dyn Handler>>` ownership pattern) but is not documented. A future maintainer could simplify the outer `Mutex` to `OnceLock` and accidentally break the "replace during tests" scenario.
- **Recommendation:** add a `//` comment beside each field explaining why the triple-wrapped shape exists. No code change needed.

## Phase-boundary check

None of F-01-001 through F-01-011 mandate a code change before Phase 2
can execute. F-01-003 is Critical but its effect is latent (no current
clone call sites); fixing it is a one-line deletion that does not
change any Phase 2 observation. The hot-file refactors (F-01-006
through F-01-009) would substantially alter later phases' surface
area but the plan (line 207) explicitly defers them until separately
approved.

**Recommendation:** continue to Phase 2 (Public API, documentation,
crates.io readiness) without pausing. If the user accepts F-01-003
(Clone removal) for immediate action, we should land it before Phase 3
so the correctness audit reflects the fixed state.
