# Engineering Diary

Purpose: durable project memory for AI-assisted development.

How to use:
- Add an entry when a noteworthy implementation lands.
- Add an entry for every bug fix, including lessons learned and prevention.
- Add an entry for important decisions and tradeoffs.
- Write entries so they still make sense if plans or temporary review docs are deleted later. Don't refer to plans and phases.
- Keep entries concise and reference concrete artifacts.
- New entries goes to the end of the file.

## Entry Template

## YYYY-MM-DD - Short title
Type: Implementation | Bug Fix | Decision
Context: Why this change happened.
Change: What was implemented/changed.
Lessons Learned: (required for Bug Fix)
Prevention: (required for Bug Fix)
Refs: path/to/file.rs, test_name, commit abc1234

## 2026-04-18 - Platform adapter architecture guidance
Type: Decision
Context: The repo-level architecture rule was phrased like an application reducer pattern, but this crate acts as a Win32 platform adapter.
Change: Updated `Agents.md` to define the architectural boundary as `native input -> AppEvent -> host state/update logic -> PlatformCommand -> native effect/render`, and aligned testing guidance with that boundary.
Refs: Agents.md

## 2026-04-18 - Style ownership and event-thread contract clarified
Type: Bug Fix
Context: `ParsedControlStyle` could be accidentally cloned despite owning GDI handles, and key platform-threading assumptions were only implicit in the code.
Change: Removed `Clone` from `ParsedControlStyle`, documented the UI-thread event contract on `PlatformEventHandler`, and normalized `listbox_handler` styling imports.
Lessons Learned: Small boundary-focused fixes are worth landing early when they remove future footguns without forcing a wider refactor.
Prevention: Treat native-resource owner types and thread-affinity contracts as explicit review items, not implicit assumptions.
Refs: src/styling_windows.rs, src/types.rs, src/app.rs, src/controls/listbox_handler.rs

## 2026-04-18 - Crate publish surface tightened for first release
Type: Decision
Context: The phase 2 API/release review found that the crate was close to publishable but still exposed an inconsistent root API, shipped author-only files in `cargo package`, and rendered sparse docs.rs output despite existing prose in source comments.
Change: Curated `Cargo.toml` package metadata and include rules, switched the declared license to match the shipped MIT license file, added docs.rs target metadata plus a CI workflow and compiled example, re-exported the remaining public support types from `src/lib.rs`, standardized public identifier wrappers on constructor/accessor helpers, and converted key public prose to rustdoc-visible comments.
Refs: Cargo.toml, src/lib.rs, src/types.rs, src/styling_primitives.rs, src/error.rs, src/app.rs, Readme.md, examples/hello_window.rs, .github/workflows/ci.yml, CHANGELOG.md

## 2026-04-19 - Win32 callback hardening and control state ownership fixes
Type: Bug Fix
Context: The phase 3 correctness review found undefined-behavior risk at Rust/Win32 callback boundaries plus latent `GWLP_USERDATA` ownership hazards in custom controls and menus.
Change: Added a shared FFI panic barrier, caught host panics inside `send_event`, moved list box and tab bar state transfer to `WM_NCCREATE`, wrapped menu handles in RAII ownership until attachment, switched panel/dark-border subclassing to `SetWindowSubclass`, and split LPARAM helpers into explicit unsigned-size and signed-coordinate variants.
Lessons Learned: Win32 callback safety depends as much on ownership transfer timing and panic boundaries as on the local message logic itself.
Prevention: Review every new `extern "system"` entry point and every `GWLP_USERDATA` write for explicit unwind handling and single-owner state transfer before landing control code.
Refs: src/ffi_safety.rs, src/app.rs, src/window_common.rs, src/controls/listbox_handler.rs, src/controls/tab_bar_handler.rs, src/controls/menu_handler.rs, src/controls/dark_border.rs, src/controls/panel_handler.rs

## 2026-04-19 - Pedantic lint debt trimmed around style copies and Win32 casts
Type: Bug Fix
Context: The phase 4 code-quality review identified a few low-risk issues that were cheap to fix directly: stale `dead_code` attributes, noisy `redundant_pub_crate` warnings, real `usize -> i32` truncation risks at Win32 boundaries, and unnecessary `Color` clones in paint-adjacent code.
Change: Derived `Copy` for `Color` and `FontWeight`, removed `clone()` calls that were only copying style colors, introduced `src/win32_cast.rs` for named saturating Win32 conversions, removed inert `#[allow(dead_code)]` annotations from public enums, and documented the crate-wide `redundant_pub_crate` policy in `src/lib.rs`.
Lessons Learned: The best way to keep pedantic lint runs useful is to fix the genuinely risky conversions and silence only the noise that encodes an intentional project convention.
Prevention: When a review item points at a specific cast or lint family, add a named helper or crate-level policy comment rather than spreading ad hoc `as` casts and `#[allow]`s through the codebase.
Refs: src/styling_primitives.rs, src/lib.rs, src/types.rs, src/win32_cast.rs, src/controls/listbox_handler.rs, src/controls/treeview_handler.rs, src/controls/button_handler.rs, docs/Review.04-CodeQuality.md, docs/Review.Backlog.md

## 2026-04-20 - Testability pass: 51 new unit tests across 6 modules
Type: Implementation
Context: The crate had 138 passing tests but several pure-logic surfaces had zero coverage — form validation predicates, dialog template builders, progress-bar clamping, toggle-switch state transitions, event-translation handlers, line-ending normalization, and the TreeView bi-directional ID map contract.
Change: Added 51 tests (138 → 189) across six areas:
1. **Dialog validation & path safety** — 8 tests for `is_safe_path_segment` and `form_validation_is_valid` covering traversal markers, separators, empty/whitespace, absolute paths, and trimming semantics.
2. **Dialog template builders** — 8 tests for all four `build_*_dialog_template` functions asserting control counts (`cdit`), expected UTF-16 class/text strings, and DWORD alignment.
3. **`pathbuf_from_buf`** — 5 tests for null-terminated, unterminated, empty, null-only, and surrogate-pair UTF-16 buffers.
4. **Progress bar & toggle switch** — Extracted `clamp_progress_range`/`clamp_progress_position` into pure functions (6 tests). Added `ToggleSwitchState::toggle()` and refactored the WndProc to use it (3 tests).
5. **Event-translation reducers** — Extracted 7 pure `translate_*` functions from the Win32 `GetDlgCtrlID`-entangled handler methods in `window_common.rs`, each with a matching test. Covers listbox selection/scroll/keydown, splitter dragging/drag-ended, toggle-switch toggled, and tab-bar selection.
6. **Line-ending normalization** — Extracted `normalize_line_endings_for_edit_control` / `normalize_line_endings_from_edit_control` from the exclude-patterns dialog proc (7 tests including round-trip).
7. **TreeView map contract** — Added `register_item`, `lookup_htreeitem`, `lookup_item_id`, `clear_maps` methods to `TreeViewInternalState`; 4 tests covering bidirectional round-trip, unknown-ID lookup, clear, and re-registration.
Refs: src/controls/dialog_handler.rs, src/controls/progress_handler.rs, src/controls/toggle_switch_handler.rs, src/controls/treeview_handler.rs, src/window_common.rs

## 2026-05-16 - Edit keydown events and explicit focus commands
Type: Feature
Context: Host apps needed a generic way to react to keys pressed inside edit controls and to move focus/select text without adding app-specific Win32 code to the toolkit.
Change: Added `AppEvent::InputKeyDown` with modifier state, an edit-control subclass that forwards `WM_KEYDOWN` through the existing app-event flow, and `PlatformCommand::SetFocus { select_all }` with a command-executor seam for optional full-text selection. Bumped the crate to 2.3.0 and covered the pure translation/focus-selection contracts with unit tests.
Refs: src/types.rs, src/controls/input_handler.rs, src/window_common.rs, src/command_executor.rs, src/app.rs, CHANGELOG.md, Cargo.toml

## 2026-05-30 - Headless harness scaffolding and checkpoint markers
Type: Feature
Context: Phase 1 of the headless backend work needed an always-compiled Rust model that can mirror a subset of platform commands, pump follow-up events, and expose stable JSON snapshots for in-process tests.
Change: Added the cross-platform `headless` module with a deterministic UI model, `HeadlessHarness`, `Checkpoint` marker recording, setup-complete follow-up delivery, and a demo-style integration test that exercises the app-core split on non-Windows builds.
Refs: src/headless.rs, src/types.rs, src/app.rs, src/lib.rs, examples/hello_window.rs

## 2026-05-30 - Headless interaction completeness for modal dialogs
Type: Feature
Context: Phase 2a needed in-process condition waits and modal-dialog fidelity so headless tests could drive the same interaction flow as the Win32 backend.
Change: Added `HeadlessHarness::wait_until`, dialog responder scripting, structured request capture in snapshots, headless completions for the save/open/profile/input/exclude-patterns/form/folder dialogs, and trace-only message-box recording; also hardened `inject_raw` as the raw-event escape hatch.
Refs: src/headless.rs, src/lib.rs, CHANGELOG.md, Cargo.toml, docs/Roadmap.HeadlessRenderBackend.md

## 2026-05-30 - Headless logical state coverage
Type: Implementation
Context: Phase 2b needed the headless interpreter to cover the remaining logical-state commands instead of leaving tree/menu/chart/style/scroll work in the unsupported bucket.
Change: Extended `src/headless.rs` with TreeView hierarchy state, chart snapshots, window menus, style application markers, scroll positions, tree selection parity, menu-action dispatch, and the public harness actions for tree selection/toggle, menu clicks, and semantic scrolling; added snapshot coverage for the new model fields.
Refs: src/headless.rs, src/types.rs, src/styling_primitives.rs, CHANGELOG.md, Cargo.toml

## 2026-05-31 - Headless stdio protocol for external drivers
Type: Implementation
Context: Shipped applications need a machine-drivable headless mode so external harnesses can inspect and drive the same app-core flow without linking Rust tests directly.
Change: Added `HeadlessHarness::run_protocol` with versioned hello/snapshot/action/wait_for/error/marker/bye JSON-lines envelopes, cursor-relative protocol waits, marker and writer flushing, request-id recovery for malformed requests, stable-name serialization for externally visible snapshot enums, and a cross-platform `--headless` example path. The protocol snapshot now omits cumulative `markers` and `quitting`; external clients use `marker` lines and `bye` as the sources of truth, while in-process snapshots retain those fields for Rust-side tests.
Refs: src/headless.rs, src/types.rs, examples/hello_window.rs, CHANGELOG.md, Cargo.toml, docs/Spec.HeadlessRenderBackend.md, docs/Roadmap.HeadlessRenderBackend.md

## 2026-06-01 - Protocol dialog scripting for headless drivers
Type: Implementation
Context: External headless drivers needed a way to pre-script modal dialog outcomes instead of always falling back to the default cancel/none behavior.
Change: Added a protocol v2 `set_dialog_responder` request plus headless-owned matcher/outcome DTOs that reconstruct the existing dialog responder script, including form field values, and validated the kind pairing before installing the script. Review follow-up unified file/folder outcomes on the external `path` field, made `message_box` entries validate but stay out of the FIFO so they cannot block later dialogs, documented replace/install-before-action semantics near the request, and added protocol tests for scripted form completion, mismatched and unknown kinds, malformed script recovery, nonmatching-script default cancel, and message-box no-op handling. Bumped the crate version to 2.8.0.
Refs: src/headless.rs, CHANGELOG.md, Cargo.toml

## 2026-06-01 - Headless module split started
Type: Implementation
Context: The headless backend had grown large enough that its tests, stable JSON snapshot DTOs, and command interpreter obscured the harness and protocol flow.
Change: Moved the extracted headless tests into `src/headless/tests.rs`, moved the `*Snapshot` DTOs and their `From` serializers into `src/headless/snapshot.rs`, moved `HeadlessBackend` plus its command handlers/validators into `src/headless/backend.rs`, moved JSON-lines protocol wire DTOs/conversions into `src/headless/protocol.rs`, and moved the window/control/tree/menu/chart state model into `src/headless/state.rs`. Visibility remains scoped to the parent module so the public `headless` API is unchanged.
Refs: src/headless.rs, src/headless/backend.rs, src/headless/protocol.rs, src/headless/snapshot.rs, src/headless/state.rs, src/headless/tests.rs

## 2026-06-02 - Headless contract catalog seeded
Type: Implementation
Context: The headless backend needed a durable way to prevent Win32/headless fidelity drift as the contract surface grows.
Change: Added `src/contracts.rs` as the shared pure-contract home, extracted `validate_layout_rules` so Win32 and headless call the same validator, seeded an append-only contract catalog in tests, and added headless parity/pinning tests for programmatic `Set*` silence plus state mutation, the `SetTreeViewSelection` event exception, disabled listbox row selection, modal completion ordering, and `ExpandVisibleTreeItems` expanding the full logical tree. The shared validator also makes Win32's multi-parent layout-violation error ordering deterministic. No public API changed.
Refs: src/contracts.rs, src/window_common.rs, src/headless/backend.rs, src/headless/tests.rs, docs/Roadmap.HeadlessRenderBackend.md

## 2026-06-02 - Headless broader input vocabulary
Type: Implementation
Context: Phase 3c needed parity for the remaining Win32 user-input events that had no headless action yet: listbox scroll, listbox key-down, and input key-down.
Change: Added `HeadlessHarness::scroll_listbox`, `HeadlessHarness::key_listbox`, and `HeadlessHarness::key_input` plus the corresponding protocol v3 action requests, wired them through the headless dispatcher with read-only inputs allowed for key-down events, and added protocol/back-end tests that pin the event stream and snapshot stability. Implementation review removed an accidental trio of dead public `PlatformCommand` variants so the feature remains a harness/protocol action surface, not a command-stream API. Bumped the crate to `2.9.0`.
Refs: Cargo.toml, CHANGELOG.md, src/headless.rs, src/headless/backend.rs, src/headless/protocol.rs, src/headless/tests.rs, src/contracts.rs
