# Engineering Diary

Purpose: durable project memory for AI-assisted development.

How to use:
- Add an entry when a noteworthy implementation lands.
- Add an entry for every bug fix, including lessons learned and prevention.
- Add an entry for important decisions and tradeoffs.
- Write entries so they still make sense if plans or temporary review docs are deleted later.
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
Refs: src/headless.rs, src/types.rs, src/app.rs, src/lib.rs, examples/hello_window.rs, tests/headless_harness.rs
