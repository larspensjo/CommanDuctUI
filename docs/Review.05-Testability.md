# Review 05 — Testability & coverage

Phase 5 findings from [Plan.ThoroughReview](Plan.ThoroughReview.md). Each
finding is recorded once here and mirrored in
[Review.Backlog.md](Review.Backlog.md).

## Audit coverage

This phase walked the crate with a testability lens:

- inventoried every `#[test]` and mapped it to the production symbol under
  test (`cargo test` → 138 passed; 0 failed; 1 doctest ignored; 0 s);
- tabulated every `PlatformCommand` variant against whether its dispatch
  path or handler logic has any unit coverage;
- scanned each `controls/*` handler for pure logic currently entangled
  with Win32 APIs that could be extracted and tested in isolation — per
  the [Agents.md](../Agents.md) guidance to prefer tests of event
  translation, emitted effects, and public contracts;
- identified surfaces that remain genuinely hard to cover without a
  running Win32 host (message loop, modal dialog procs, custom-draw
  against real HDCs) and noted why.

The entry points examined for decomposition hints are the four hot files
flagged in [Review.00-Survey.md](Review.00-Survey.md):
[window_common.rs](../src/window_common.rs),
[controls/treeview_handler.rs](../src/controls/treeview_handler.rs),
[controls/dialog_handler.rs](../src/controls/dialog_handler.rs),
[app.rs](../src/app.rs), plus
[command_executor.rs](../src/command_executor.rs).

## Scorecard

What looks solid:

- **Test discipline is established.** 138 inline tests across 20
  modules, all passing. No skipped / ignored integration tests. Target
  name is `commanductui` and tests compile on Windows in <1 s.
- **Pure computation is generally extracted.** The chart module tests
  its tick/stride/label-placement functions
  ([chart_handler.rs:155-233](../src/controls/chart_handler.rs#L155));
  the tab bar tests `hit_test` and `next_selected_index_for_key`
  ([tab_bar_handler.rs:388-405](../src/controls/tab_bar_handler.rs#L388));
  the button handler tests `blend_color` and
  `resolve_disabled_button_colors`
  ([button_handler.rs:333-340](../src/controls/button_handler.rs#L333)).
- **Layout is fully decoupled from HWNDs.**
  `NativeWindowData::calculate_layout` is a pure function over
  `(RECT, &[LayoutRule])`, covered by five geometry tests plus four
  validator tests in
  [window_common.rs:3573-3709](../src/window_common.rs#L3573).
- **TreeView colour resolution is comprehensively covered.** Seven
  tests on `resolve_item_colors` cover the base / selection /
  per-item-override cross-product, and the custom-draw post-paint
  decision is tested via `should_request_postpaint` and
  `should_draw_selection_accent`
  ([treeview_handler.rs:1696-2005](../src/controls/treeview_handler.rs#L1696)).
- **Good extraction seam example.**
  `read_edit_control_text_with<FLen, FGet>` in
  [window_common.rs:3041](../src/window_common.rs#L3041) accepts
  injectable length/read closures so the buffer-growth logic is
  directly unit-tested without Win32. This is the pattern the rest of
  the crate should imitate; see F-05-003 and F-05-004.
- **Panic / poisoning contracts have regression tests.** The
  F-03-001 fix has
  `send_event_catches_handler_panics_without_poisoning_handler_mutex`
  and `send_event_ignores_poisoned_handler_registry`
  ([app.rs:1569-1620](../src/app.rs#L1569)); FFI unwind protection has
  `catch_unwind_ffi_returns_default_after_panic`
  ([ffi_safety.rs:86](../src/ffi_safety.rs#L86)).

What needs work: the findings below.

## `PlatformCommand` coverage matrix

51 variants, grouped by the quality of existing coverage. "Error path"
means the handler has a unit test that exercises the missing-control /
missing-parent rejection branch but not the happy path. "Pure slice"
means a pure function called by the handler is tested, but the handler
itself is not. "None" means no test references the variant at all.

| Coverage | Variants |
|---|---|
| **Handler has error-path test** | `SetControlEnabled`, `SetControlText`, `SetRichEditContent`, `CreateInput`, `CreateRichEdit` (missing-parent + duplicate-id), `ExpandVisibleTreeItems`, `ExpandAllTreeItems` |
| **Happy-path via inline test** | `DefineLayout` (stored rules tested via `define_layout_*`), `DefineStyle` (via `define_style_stores_parsed_style`), `ApplyStyleToControl` (via `test_apply_style_to_control_records_id`), `UpdateLabelText` (via `test_set_and_get_label_severity`) |
| **Pure slice tested, handler untested** | `ShowMessageBox` (icon-flag map only), `CreateButton` (notification translation), `CreateCheckBox` (style-flag composition), `CreateRadioButton` (group-start flag composition), `CreateComboBox` (selection-index mapping, UTF-16 helper), `CreateTabBar` + `SetTabBarStyle` (palette derive, hit-test, arrow-key state), `CreateChart` + `SetChartData` (ticks, stride, label placement), `PopulateListBox` + `CreateListBox` (palette, badge rect, row rect, nav keys), `CreateMainMenu` (WM_COMMAND lookup, recursive menu-id builder), `CreatePanel` (style flags) |
| **None** | `SetWindowTitle`, `ShowWindow`, `CloseWindow`, `QuitApplication`, `SignalMainWindowUISetupComplete`, `PopulateTreeView`, `UpdateTreeItemVisualState`, `UpdateTreeItemText`, `RedrawTreeItem`, `SetTreeViewSelection`, `SetListBoxSelection`, `SetInputText`, `SetViewerContent`, `SetScrollPosition`, `CreateTreeView`, `CreateLabel`, `CreateProgressBar` / `SetProgressBarRange` / `SetProgressBarPosition`, `CreateSplitter`, `SetComboBoxItems`, `SetComboBoxSelection`, `SetCheckBoxChecked`, `SetRadioButtonChecked`, `SetTabBarItems`, `SetTabBarSelection`, `CreateToggleSwitch`, `SetToggleSwitchState`, `SetToggleSwitchStyle`, `ShowSaveFileDialog`, `ShowOpenFileDialog`, `ShowProfileSelectionDialog`, `ShowInputDialog`, `ShowExcludePatternsDialog`, `ShowFormDialog`, `ShowFolderPickerDialog` |

**Summary:** 7 variants have direct error-path coverage, 4 have
happy-path coverage via storage/state assertions, 18 have adjacent
pure-logic coverage, and **22 variants (43%) have no test referencing
them at all**. The untested set concentrates in three clusters:

1. Dialog commands (all 7 non-trivial variants except `ShowMessageBox`)
   — see F-05-004.
2. TreeView and ListBox mutation commands (populate, update, redraw,
   select) — see F-05-006.
3. Entire control families with zero module-level tests: progress bar,
   splitter, toggle switch — see F-05-003.

## Hard-to-test surfaces

These surfaces are hard to cover by design. They are called out so
recommendations can focus where payoff is real:

- **Main `wnd_proc`.** The central dispatcher in
  [window_common.rs](../src/window_common.rs) routes ~40 Win32 messages
  and cannot run without a registered window class + live HWND.
  Testing it requires an integration harness that creates a hidden
  window — out of scope for unit tests, and marginal value once the
  inner handlers are individually covered.
- **Modal dialog procs.** `input_dialog_proc`,
  `exclude_patterns_dialog_proc`, `form_dialog_proc`, and the profile
  selection dialog all mix state transitions with `SetDlgItemTextW` /
  `EndDialog` calls. The reducer inside each is small and should be
  extracted (F-05-005) — the proc itself stays untestable.
- **GDI custom-draw against a real HDC.** Tree view custom-draw,
  listbox owner-draw, chart paint, tab-bar paint, and toggle-switch
  paint all ultimately write to a `HDC`. The decision logic has
  already been partially extracted and tested; actual pixel output
  verification requires a bitmap DC and is a separate effort.
- **`IFileOpenDialog` / `GetSaveFileNameW` commands.** The seven file-
  and folder-picker commands call into COM/modal Win32 and run an
  inner message pump. They are unit-uncoverable. Extract the result-
  translation slice (F-05-004).
- **Dark-mode ordinal binding.** `try_enable_dark_mode` resolves
  uxtheme ordinals at runtime; the ordinal-resolution slice is
  testable via the injected predicate (and is tested), but the actual
  `LoadLibraryW` + `GetProcAddress` + dispatch path is not.

---

## Findings

### F-05-001: `progress_handler`, `splitter_handler`, `toggle_switch_handler` have zero tests

- **Severity:** Minor
- **Dimension:** testing
- **Status:** open
- **Location:** [src/controls/progress_handler.rs](../src/controls/progress_handler.rs),
  [src/controls/splitter_handler.rs](../src/controls/splitter_handler.rs),
  [src/controls/toggle_switch_handler.rs](../src/controls/toggle_switch_handler.rs)
- **Observation:** Three entire control modules ship with no
  `#[cfg(test)]` block. Of these, the progress bar has the simplest
  contract (`handle_set_progress_bar_range` clamps `max` to `>= min`
  and to `i32::MAX as u32` at
  [progress_handler.rs:120-121](../src/controls/progress_handler.rs#L120),
  and `handle_set_progress_bar_position` clamps the position at
  [progress_handler.rs:150](../src/controls/progress_handler.rs#L150)).
  The splitter has a drag-position clamp and orientation-driven cursor
  logic. The toggle switch has a `ToggleSwitchPalette::default` and a
  keyboard/click → checked transition that both lend themselves to
  pure unit tests.
- **Why it matters:** Six `PlatformCommand` variants
  (`CreateProgressBar`, `SetProgressBarRange`, `SetProgressBarPosition`,
  `CreateSplitter`, `CreateToggleSwitch`, `SetToggleSwitchState`) are
  dispatched but nothing asserts the clamping or the state transition.
  The clamping in particular is the kind of arithmetic that silently
  regresses.
- **Recommendation:** Add three small test modules. In `progress_handler`,
  extract the clamp into `fn clamp_progress_range(min, max) -> (u32,
  u32)` and test the `min > max` and `max > i32::MAX as u32` cases. In
  `toggle_switch_handler`, add `ToggleSwitchState::toggle()` (if not
  already present) and test it — the state-transition contract is the
  public one. In `splitter_handler`, test the orientation-to-cursor
  mapping and any drag-position clamp.

### F-05-002: Dialog form validation logic is pure and untested

- **Severity:** Major
- **Dimension:** testing
- **Status:** open
- **Location:** [src/controls/dialog_handler.rs:634-651](../src/controls/dialog_handler.rs#L634)
- **Observation:** `form_validation_is_valid` and `is_safe_path_segment`
  are called for every keystroke in a form dialog with a
  `FormTextValidation::PathSegment` field. They decide whether the OK
  button enables, and `is_safe_path_segment` rejects traversal markers
  (`.`, `..`, `/`, `\`, `\0`) and absolute paths. Neither has a test.
- **Why it matters:** These predicates are the first line of defence
  against host apps round-tripping unvetted user input into a
  `PathBuf`. They're pure (`&str → bool`), trivially testable, and the
  exact class of function where silent regressions cost real users.
  This is also the most security-adjacent piece of platform-layer
  logic in the crate.
- **Recommendation:** Add inline tests for the happy path (typical
  filename), each rejected marker, mixed separators, trailing
  whitespace (current behaviour via `trim()` passes `" foo "` — is
  that intentional? lock it down with a test), and
  `FormTextValidation::NonEmpty` over a whitespace-only input.

### F-05-003: Event-translation handlers mix `HWND` lookups with pure reduction

- **Severity:** Major
- **Dimension:** testing
- **Status:** open
- **Location:**
  [listbox_handler.rs](../src/controls/listbox_handler.rs) (selection /
  key-down / scroll notifications),
  [treeview_handler.rs:537-619](../src/controls/treeview_handler.rs#L537)
  (`handle_treeview_itemchanged_notification`,
  `handle_treeview_selection_changed_notification`),
  [window_common.rs](../src/window_common.rs) (splitter drag, scroll,
  tab-bar, radio, check notifications)
- **Observation:** The emission sites for 13 of 27 `AppEvent` variants
  live in `window_common.rs` and translate a raw WPARAM/LPARAM plus an
  HWND-to-ControlId lookup into an `AppEvent`. Today they call
  `SendMessageW` / `GetWindowLongPtrW` in-line — the moment you need
  an HWND you can't unit-test. Contrast with
  [button_handler.rs:183-197](../src/controls/button_handler.rs#L183):
  `handle_bn_clicked(window_id, control_id, hwnd)` takes an `HWND`
  but doesn't dereference it, which is why
  `bn_clicked_translates_to_app_event` works with
  `HWND::default()`.
- **Why it matters:** Event translation is explicitly the shape the
  crate tells hosts to care about ([Agents.md:24](../Agents.md#L24):
  "Prefer tests of event translation, emitted effects, and public
  contracts over internal details"). A subtle bug in
  `TreeViewItemSelectionChanged` (wrong `item_id` on programmatic
  vs. user selection) already needed the `TVC_BYMOUSE`/`TVC_BYKEYBOARD`
  discriminator test
  (`user_treeview_selection_action_accepts_mouse_and_keyboard`) —
  that pattern should be extended.
- **Recommendation:** For each event emission site, split the function
  in two: (a) a pure reducer
  `fn translate_<notification>(window_id, control_id, <payload>) ->
  AppEvent` and (b) a thin site that reads the payload from Win32 and
  calls the reducer. Test (a). The change is local and non-breaking.
  Start with the `ListBoxItemSelectionChanged`,
  `ListBoxScrolled`, `ControlScrolled`, and `SplitterDragging` sites,
  where the payload is numeric coordinates rather than window
  hierarchy.

### F-05-004: Dialog command result-translation has no tests

- **Severity:** Major
- **Dimension:** testing
- **Status:** open
- **Location:**
  [dialog_handler.rs:71-75](../src/controls/dialog_handler.rs#L71)
  (`pathbuf_from_buf`),
  [dialog_handler.rs:107-193](../src/controls/dialog_handler.rs#L107)
  (`show_common_file_dialog`) and the per-dialog
  `FormFieldValue`/`FormFieldRuntime` collection at
  [dialog_handler.rs:713-746](../src/controls/dialog_handler.rs#L713).
- **Observation:** The seven file/form/folder dialog commands produce
  an `AppEvent` carrying the user's selection. The Win32 call is
  unavoidably untestable, but the translation back — buffer → `PathBuf`
  via `pathbuf_from_buf`, field list → `Vec<FormFieldValue>` via
  `collect_form_field_values`, string → `PathBuf` in the folder picker
  — is pure or near-pure and has no tests.
- **Why it matters:** These are the widest-mouthed payloads the
  platform layer hands back to hosts. `pathbuf_from_buf` handles both
  null-terminated and unterminated UTF-16 slices
  ([dialog_handler.rs:72](../src/controls/dialog_handler.rs#L72));
  misreading either is a classic off-by-one.
- **Recommendation:** Test `pathbuf_from_buf` for: the null-terminated
  case, the unterminated case, an empty buffer, a buffer whose only
  content is `0x0000`, and a multi-byte UTF-16 (surrogate pair) path.
  Extract the validation+coercion logic from
  `collect_form_field_values` into a pure function taking a
  `&[(FormFieldRuntime, Option<String>, bool)]`-shaped slice and test
  it.

### F-05-005: Dialog proc state transitions are trapped inside unsafe `extern "system"` procs

- **Severity:** Minor
- **Dimension:** testing
- **Status:** open
- **Location:**
  [dialog_handler.rs:751-827](../src/controls/dialog_handler.rs#L751)
  (`input_dialog_proc`),
  [dialog_handler.rs:833-925](../src/controls/dialog_handler.rs#L833)
  (`exclude_patterns_dialog_proc`),
  and the `form_dialog_proc` block slightly further down.
- **Observation:** Each dialog proc couples four concerns in one
  function: initial text seeding (WM_INITDIALOG), input reading on
  confirm, Cancel/Close outcome bookkeeping, and `EndDialog` dispatch.
  The "what should happen on OK vs. Cancel vs. Close" is a small FSM
  that today only runs under a real modal dialog.
- **Why it matters:** The exclude-patterns proc already has a known
  round-trip quirk (`\r\n` ↔ `\n` normalisation at
  [dialog_handler.rs:858-861 and 895-896](../src/controls/dialog_handler.rs#L858)).
  That should be locked down with a test.
- **Recommendation:** Extract each proc's decision logic as
  `fn step(current: &DialogData, input: DialogMsg) -> DialogAction`
  where `DialogAction` is one of `{SeedText(String), ReadInput,
  Confirm, Cancel}`. The proc becomes a shim that translates
  WM_INITDIALOG/WM_COMMAND into `DialogMsg`, calls `step`, and
  translates `DialogAction` into `SetDlgItemTextW` / `EndDialog`.
  Unit tests cover the FSM, including the `\r\n` canonicalisation.

### F-05-006: TreeView and ListBox mutation commands have no end-to-end coverage

- **Severity:** Major
- **Dimension:** testing
- **Status:** open
- **Location:**
  [command_executor.rs:181-318](../src/command_executor.rs#L181)
  (populate / update / select),
  [treeview_handler.rs:342-888](../src/controls/treeview_handler.rs#L342)
- **Observation:** The command dispatcher maps eight tree-related and
  three listbox-related commands into `command_executor` /
  `treeview_handler` / `listbox_handler`. `execute_expand_*` has a
  missing-control error-path test; everything else (populate, update
  state, update text, redraw, set selection) is exercised only through
  the integration example.
- **Why it matters:** `PopulateTreeView` and `UpdateTreeItemText`
  maintain the `item_id_to_htreeitem` / `htreeitem_to_item_id`
  bi-directional map in `TreeViewInternalState`
  ([treeview_handler.rs:74-79](../src/controls/treeview_handler.rs#L74)).
  If that map drifts, every subsequent notification hands the host a
  wrong `TreeItemId`. This is exactly the contract
  [Agents.md:17](../Agents.md#L17) promises ("`AppEvent` → host state
  → `PlatformCommand` → native effect") and silent drift here would
  be undiagnosable from the host side.
- **Recommendation:** Split `TreeViewInternalState` mutation from
  the Win32 `TVM_INSERTITEMW` / `TVM_SETITEMW` calls, then test:
  populate-then-lookup round-trip, delete-item clears both maps,
  update-text preserves `HTREEITEM`, re-populate invalidates stale
  `TreeItemId`s. Do the same for
  `ListBoxInternalState` (selection / scroll / key-down reducers).

### F-05-007: `execute_platform_command` dispatch has no registration-level check

- **Severity:** Minor
- **Dimension:** testing
- **Status:** open
- **Location:** [app.rs:448-939](../src/app.rs#L448)
- **Observation:** The 650-line `match` on `PlatformCommand`
  (cf. [F-04-005](Review.04-CodeQuality.md#f-04-005-execute_platform_command-is-a-650-line-60-variant-match))
  is compiler-checked for exhaustiveness today, but nothing guarantees
  that each arm dispatches to a real handler rather than a silent stub
  added during a refactor. `PlatformCommand` is `#[non_exhaustive]` to
  external callers only — inside the crate the match must still be total.
- **Why it matters:** There's no automated reminder that every variant
  still has a real handler. However, a naive fix — constructing one
  `PlatformCommand` per variant and calling `execute_platform_command`
  on each — is *not* safe today: seven dialog variants dispatch
  synchronously into modal Win32 / COM calls
  (`GetSaveFileNameW`, `GetOpenFileNameW`, `DialogBoxIndirectParamW`,
  `MessageBoxW`, `IFileOpenDialog::Show`) in
  [dialog_handler.rs:154-244](../src/controls/dialog_handler.rs#L154),
  [541-544](../src/controls/dialog_handler.rs#L541),
  [1456-1458](../src/controls/dialog_handler.rs#L1456),
  [1736-1792](../src/controls/dialog_handler.rs#L1736), and
  [1873-1898](../src/controls/dialog_handler.rs#L1873). Calling them
  from a unit test would hang or pop a UI.
- **Recommendation:** Two paths, pick one:
  1. **Smaller:** add a smoke test over the *non-modal* subset
     explicitly (hand-maintained `const MODAL_VARIANTS: &[&str]` whose
     names are skipped). Accepts that the list must be kept in sync
     when new modal commands land, so pair with a comment at the
     `PlatformCommand` definition.
  2. **Bigger but durable:** refactor dispatch into a table
     (`fn(PlatformCommand, &Arc<Win32ApiInternalState>) ->
     PlatformResult<()>` per variant, or a `HandlerKind` enum that
     tags modal vs. non-modal). The table is then enumerable: the
     test walks the table and asserts every variant has an entry,
     without calling through to the modal handlers. This also
     addresses [F-04-005](Review.04-CodeQuality.md#f-04-005-execute_platform_command-is-a-650-line-60-variant-match).

  Either way, make the modal-command caveat explicit in the test —
  otherwise the next person to copy the pattern rediscovers the
  hang the hard way.

### F-05-008: Known-incident regressions lack guard tests

- **Severity:** Minor
- **Dimension:** testing
- **Status:** open
- **Location:** varies — see [Review.Backlog.md](Review.Backlog.md)
- **Observation:** Of the Phase 1–3 findings already fixed, the
  regressions with tests are: F-01-002 (send_event synchrony),
  F-01-003 (ParsedControlStyle Clone — covered by compile-check only),
  F-03-001 (panic handling has `send_event_catches_handler_panics…`
  and the `ffi_safety::tests` pair). The fixed items **without** a
  dedicated regression test are F-02-001 (license file content),
  F-02-014 (`cargo package` inclusion list), F-03-002 / F-03-003
  (post-create GWLP_USERDATA overwrite), F-03-004 (HMENU leak on
  error path), F-03-005 (LOWORD/HIWORD sign extension — *is* tested
  by `lparam_coordinate_helpers_sign_extend_negative_positions`),
  F-03-006 (subclass GWLP_USERDATA aliasing).
- **Why it matters:** [Agents.md:24](../Agents.md#L24) asks for a
  regression test "when practical" on bug fixes. Several of the
  Phase 3 fixes are practical: F-03-004 can be guarded by a test that
  runs the error branch and verifies the menu handle was destroyed
  (via `GetLastError` or a mock handle registry); F-03-002 /
  F-03-003 can be guarded by a test that inserts a fake HWND into
  `GWLP_USERDATA` and asserts the post-create path no longer
  overwrites it.
- **Recommendation:** For each open gap above, decide at backlog
  review whether a test is "practical" or whether the compile-time
  check is sufficient (as it is for F-01-003's `Clone` removal).

### F-05-009: No Win32-hosted integration coverage of `PlatformInterface::run`

- **Severity:** Nit
- **Dimension:** testing
- **Status:** open
- **Location:** repository root
- **Observation:** Inline unit tests are the repo's stated preference
  ([Agents.md:25-27](../Agents.md#L25)) and directory layout is not a
  finding in itself. The real gap is that no test — inline or
  otherwise — drives `PlatformInterface::run` end-to-end. The only
  example, `examples/hello_window.rs`, exercises a single window
  lifecycle by hand but is not executed by `cargo test`.
- **Why it matters:** `PlatformInterface::run` is the crate's primary
  entry point, and nothing in the 138 unit tests touches the
  command-drain → message-loop → event-emission round trip. A host
  wiring `AppEvent` ↔ `PlatformCommand` incorrectly would not be
  caught until a human launched a binary.
- **Recommendation:** Add one Win32-hosted integration test (whether
  inline under `#[cfg(target_os = "windows")]` or in a `tests/`
  binary is a detail — inline is fine) that drives a short command
  sequence (create window → create button → post click notification
  → assert emitted `AppEvent` → close). Keep it to one test so CI
  latency doesn't regress.

### F-05-010: Test scaffolding is duplicated across modules

- **Severity:** Nit
- **Dimension:** testing
- **Status:** open
- **Location:**
  [command_executor.rs:693-699](../src/command_executor.rs#L693),
  [window_common.rs:3439+](../src/window_common.rs#L3439),
  [app.rs:1569+](../src/app.rs#L1569)
- **Observation:** `Win32ApiInternalState::new("TestAppForExecutor")`
  / `NativeWindowData::new(window_id)` / `active_windows().write()` is
  cut-and-pasted across test modules. The `setup_test_env` helper in
  `command_executor` is identical in shape to blocks in `app` and
  `window_common`.
- **Why it matters:** The duplication is small today but it raises
  the friction of writing the tests recommended in F-05-001 and
  F-05-006. A one-time consolidation would make writing a happy-path
  command test trivially cheap.
- **Recommendation:** Expose a `pub(crate)` test-only helper under a
  `#[cfg(test)] mod test_support;` in `app.rs` that returns
  `(Arc<Win32ApiInternalState>, WindowId, NativeWindowData)` and
  optionally registers the data. Re-export a `register_fake_control`
  helper from `NativeWindowData`. This is not part of the public API.

### F-05-011: Dialog template builders are pure and untested

- **Severity:** Major
- **Dimension:** testing
- **Status:** open
- **Location:**
  [dialog_handler.rs:398](../src/controls/dialog_handler.rs#L398)
  (`build_profile_dialog_template`),
  [dialog_handler.rs:951](../src/controls/dialog_handler.rs#L951)
  (`build_form_dialog_template`),
  [dialog_handler.rs:1507](../src/controls/dialog_handler.rs#L1507)
  (`build_input_dialog_template`),
  [dialog_handler.rs:1610](../src/controls/dialog_handler.rs#L1610)
  (`build_exclude_patterns_dialog_template`).
- **Observation:** Four functions assemble `DLGTEMPLATE` byte buffers
  from descriptors alone — no HWND, no COM object, no message loop.
  They write the header, item count, class names, control IDs, text
  payloads, and DWORD alignment padding via the
  `push_word` / `push_str_utf16` / `align_to_dword` helpers at
  [dialog_handler.rs:1485-1504](../src/controls/dialog_handler.rs#L1485).
  Despite being the most test-friendly code in `dialog_handler.rs`,
  none of them has a test. The module's only existing test is
  `icon_flag_tracks_severity` on the six-line message-box helper.
- **Why it matters:** A malformed template breaks dialog creation
  before the proc/FSM logic covered by F-05-005 or the result
  translation covered by F-05-004 ever runs — the failure mode is
  "dialog doesn't appear" with no host-visible cause. The builders
  also encode subtle invariants: the form builder saturates item
  counts at `u16::MAX`, the template must be DWORD-aligned between
  items, and `DS_SETFONT` / `DS_CENTER` style bits must be set
  together for the system font selection to take effect.
- **Recommendation:** For each builder, add tests that assert:
  (a) `cdit` matches the number of control items the descriptor
  implies (text input → 2, text input with live warning → 3,
  checkbox → 1, plus OK/Cancel); (b) the byte buffer is
  DWORD-aligned at every item boundary; (c) expected control IDs
  appear in the expected order; (d) class names (`WC_EDIT`,
  `WC_BUTTON`, `WC_STATIC`) appear at the right offsets for each
  item kind; (e) supplied text payloads are present in UTF-16. These
  assertions run on the in-memory `Vec<u8>` — no Win32 involved.

---

## Recommendations ordered by payoff

From highest-leverage to lowest. Each recommendation maps to the
finding that describes its full rationale.

1. **Test dialog form validation predicates** (F-05-002). Smallest
   change, highest user-visible risk: `form_validation_is_valid` and
   `is_safe_path_segment` are pure and ~10 lines each.
2. **Test the four dialog template builders** (F-05-011). Pure
   byte-builders, no Win32 needed, and a malformed template silently
   prevents a dialog from ever showing.
3. **Extract and test event-translation reducers** (F-05-003).
   Aligns directly with the repo's stated testing preference. Start
   with listbox, scroll, and splitter; treeview selection already
   has a partial reducer.
4. **Unit-test `TreeViewInternalState` / `ListBoxInternalState`
   mutations** (F-05-006). These are the bi-directional ID maps that
   guarantee `AppEvent` payloads reference host IDs rather than stale
   native handles.
5. **Extract and test dialog proc FSMs** (F-05-005). Locks down the
   `\r\n` canonicalisation and every OK/Cancel outcome without
   needing a running dialog.
6. **Test `pathbuf_from_buf` and the form-field value collection
   reducer** (F-05-004). The widest-mouthed payload produced by the
   platform layer.
7. **Add baseline tests to `progress_handler`,
   `splitter_handler`, `toggle_switch_handler`** (F-05-001).
8. **Add a dispatch-registration check** (F-05-007). Either a
   non-modal-subset smoke test or a dispatch-table refactor —
   *not* the naive "call every variant" test, which would hang on
   modal commands.
9. **Consolidate test scaffolding** (F-05-010). Unblocks #3, #4, and
   #7 from feeling expensive to write.
10. **Add one Win32-hosted integration test** (F-05-009). Protects
    `PlatformInterface::run`.
11. **Add regression tests for the fixed Phase 3 findings where
    practical** (F-05-008). Smallest marginal gain but good
    discipline.

These recommendations produce no new findings on their own — they
describe the work that F-05-001 through F-05-010 imply. Execution is
out of scope for Phase 5.
