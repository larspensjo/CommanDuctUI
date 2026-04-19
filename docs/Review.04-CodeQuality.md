# Review 04 — Code quality & idiomatic Rust

Phase 4 findings from [Plan.ThoroughReview](Plan.ThoroughReview.md). Each
finding is recorded once here and mirrored in
[Review.Backlog.md](Review.Backlog.md).

This revision incorporates the corrections in
[Review.04-Audit.md](Review.04-Audit.md):
- F-04-001 adds `tab_bar_handler` to its evidence and retires the
  "user-object ceiling" mischaracterisation in favour of the GDI-object
  quota.
- F-04-005 uses the actual function length (~650 lines, not 350).
- F-04-008 replaces the test-only citations with four real production
  `usize → i32` truncation sites in `treeview_handler` and
  `listbox_handler`.
- F-04-010 is tightened — no longer implies `CreateWindowExW` runs
  inside the flagged guards.
- F-04-012 and F-04-013 are relocated to a separate "API design
  questions" section and labelled as lower-confidence design topics
  rather than defects.
- Scorecard exact counts are replaced with approximate shares;
  re-run clippy before citing.

## Audit coverage

This phase walked the crate with a code-quality / idiomatic-Rust lens:

- `cargo clippy --all-targets -- -W clippy::pedantic -W clippy::nursery`
  (on the order of 900 warnings across the tree — 884 on `lib`, 920 on
  `lib test`; baseline `cargo clippy --all-targets` is clean);
- error-handling patterns — `PlatformError::*(format!(…))` vs. structured
  variants, `let _ = …` on Win32 return codes, silent fallbacks in paint
  paths;
- DRY review across the per-control handlers and the `command_executor`
  → per-handler dispatch seam;
- dead-code scan — `#[allow(dead_code)]` annotations, pub surfaces
  without callers;
- allocation hotspots in the paint / message paths
  (`paint_list_box`, `paint_chart`, `paint_tab_bar`,
  custom-draw TreeView);
- trait ergonomics for `PlatformEventHandler` and `UiStateProvider`
  (the two host-facing traits).

## Scorecard

What looks solid:

- Baseline `cargo clippy --all-targets` (no extra lints) is clean, so
  the crate has no live lint debt beyond the pedantic/nursery tiers.
- Error propagation is consistent: every fallible platform function
  returns `PlatformResult<T>` and callers use `?`. There is no silent
  `unwrap`/`expect` on public error paths (see Phase 3
  [F-03-001](Review.03-Correctness.md#f-03-001-host-handle_event-panic-can-unwind-across-the-ffi-boundary-ub)
  for the one residual mutex `.unwrap()`, already fixed).
- Win32 return codes that the crate intentionally ignores are spelled
  with `let _ = unsafe { … };` — this is load-bearing idiom in
  `command_executor` and the paint paths.
- `SelectedObject::select` ([controls/gdi_utils.rs:16](../src/controls/gdi_utils.rs#L16))
  is a clean RAII seam used consistently for GDI selection restore.
- Rustdoc on public items is now complete (Phase 2
  [F-02-003](Review.02-ApiAndRelease.md#f-02-003-no-crate-level-rustdoc-most-public-items-use-non-rustdoc-comments)).

Clippy warning composition (pedantic + nursery) — approximate shares on
the lib run; exact counts drift with each lint release and should be
re-run before citing:

| Rough count | Lint | Commentary |
|------------:|------|------------|
| ~170 | `redundant_pub_crate` | bulk cosmetic — see F-04-007 |
| ~230 | `cast_*` family (truncation/wrap/sign_loss/lossless) | mostly Win32 API coercions — F-04-008 |
| ~110 | `borrow_as_ptr` | `&x as *const _` → `std::ptr::from_ref(&x)` |
|  ~55 | `missing_const_for_fn` | easy win, no semver impact |
|  ~25 | `doc_markdown` | identifiers not in backticks |
|  ~20 | `needless_pass_by_value` | idiomatic — F-04-009 |
|  ~20 | `too_many_lines` | echoes Phase 1 hot-file findings |
|  ~18 | `items_after_statements` | local-scope cleanup |
|  ~10 | `significant_drop_tightening` | see F-04-010 |
|    7 | `unused_self` | `self: &Arc<Self>` on stateless methods — F-04-011 |

## Findings

### F-04-001: Per-row `CreateSolidBrush` / `DeleteObject` churn in paint paths
- **Severity:** Major
- **Dimension:** quality
- **Status:** open
- **Location:** [src/controls/listbox_handler.rs:770-808](../src/controls/listbox_handler.rs#L770), [src/controls/listbox_handler.rs:895-899](../src/controls/listbox_handler.rs#L895), [src/controls/chart_handler.rs:397-421](../src/controls/chart_handler.rs#L397), [src/controls/tab_bar_handler.rs:449-515](../src/controls/tab_bar_handler.rs#L449), [src/controls/treeview_handler.rs:1328-1493](../src/controls/treeview_handler.rs#L1328)
- **Observation:** Every visible list-box row allocates and frees **at least two** `HBRUSH`es per paint (`row_brush`, optional `accent`, plus one brush per visible badge); `paint_chart` creates and frees a `HPEN` + `HBRUSH` per plotted bar / line segment; `paint_tab_bar` creates and frees a background, optional hover, and accent brush on every paint; the TreeView custom-draw handler allocates a `bg_brush` per item on every `NM_CUSTOMDRAW`. Palettes and line colors are stable across paints, so these are effectively cache-misses by construction. There is no `HBRUSH` cache on `ListBoxState` / `TabBarState` / `NativeWindowData`; every frame hits the GDI allocator.
- **Why it matters:** `CreateSolidBrush`/`DeleteObject` are cheap individually, but WM_PAINT runs on the UI thread and the listbox paints the full viewport on every `WM_SIZE`, scroll, hover, and focus change. A 30-row list with three badges/row produces ~120 brush allocations per paint; the chart view adds ~20–40 more per redraw. Beyond the wall-clock cost, the per-paint churn adds steady pressure to the per-process GDI-object quota and makes profiler traces noisy. The Phase 3 audit already confirmed each `CreateSolidBrush` is paired with `DeleteObject`, so this is a performance finding, not a leak.
- **Recommendation:** add a small `BrushCache` keyed on `COLORREF` to each state struct that owns a palette. Simplest shape: a `smallvec::SmallVec<[(COLORREF, HBRUSH); 8]>` behind a `fn get_or_create(color) -> HBRUSH`, cleared on palette change and drained in `Drop`. Apply to `ListBoxState`, `TabBarState`, `ChartState`, and `NativeWindowData::get_treeview_new_item_font`-style helpers. No public-API impact.

### F-04-002: Strings re-encoded to UTF-16 per frame inside paint loops
- **Severity:** Minor
- **Dimension:** quality
- **Status:** open
- **Location:** [src/controls/listbox_handler.rs:839](../src/controls/listbox_handler.rs#L839), [src/controls/listbox_handler.rs:853](../src/controls/listbox_handler.rs#L853), [src/controls/listbox_handler.rs:879](../src/controls/listbox_handler.rs#L879), [src/controls/chart_handler.rs:289](../src/controls/chart_handler.rs#L289), [src/controls/chart_handler.rs:342](../src/controls/chart_handler.rs#L342)
- **Observation:** `paint_list_box` rebuilds `Vec<u16>` buffers for `item.title` and `item.metadata` on every row on every frame; `draw_badges` does the same for every visible badge; `paint_chart` format-strings ticks (`format!("{t}")`) and then encodes to UTF-16 per paint. The source `String`s are stored in the state struct and mutate only through `PopulateListBox` / `PopulateChart` commands, so the UTF-16 encoding is cacheable.
- **Why it matters:** allocates `Vec<u16>` + heap `String` (in the chart case) on every paint without changing output; trivially wasteful and contributes to the paint-path GC pressure flagged above. The `format!("{t}")` in chart paint is particularly egregious since `t: u32` — a stack-buffer formatter (or `itoa`) eliminates the allocation entirely.
- **Recommendation:** precompute `title_utf16: Vec<u16>` and `metadata_utf16: Vec<u16>` at populate time on `ListBoxItemDescriptor` (internal mirror, not public), and `ticks_utf16: Vec<Vec<u16>>` when the chart ticks are built. For the chart ticks specifically, replace `format!("{t}")` with `itoa::Buffer` → `str::encode_utf16()` collected once per tick.

### F-04-003: `PlatformError` construction is stringly-typed and duplicated
- **Severity:** Major
- **Dimension:** quality
- **Status:** open
- **Location:** 128 sites across `src/command_executor.rs`, `src/window_common.rs`, `src/controls/*` — e.g. [command_executor.rs:104](../src/command_executor.rs#L104), [command_executor.rs:155](../src/command_executor.rs#L155), [command_executor.rs:346](../src/command_executor.rs#L346), [command_executor.rs:456](../src/command_executor.rs#L456), [treeview_handler.rs:157-850](../src/controls/treeview_handler.rs#L157)
- **Observation:** Every handler builds `PlatformError::InvalidHandle(format!("Control ID {} not found for … in WinID {:?}", control_id.raw(), window_id))` inline. The shape repeats verbatim across ~20 call sites (InvalidHandle for missing-control, OperationFailed for Win32-call-failure, ControlCreationFailed for CreateWindowExW failure). The message template is duplicated, not constructed from a shared helper.
- **Why it matters:** three concrete costs:
  1. Every edit to message phrasing has to be made at 128 sites.
  2. Structured downstream handling is impossible — consumers would have to parse the message string to extract `window_id` / `control_id`. This directly blocks the Phase 2
     [F-02-012](Review.02-ApiAndRelease.md#f-02-012-platformerror-variants-discard-structured-cause-information)
     recommendation.
  3. It hides the one genuinely interesting mismatch — `execute_set_viewer_content` / `execute_set_input_text` deliberately share `execute_set_control_text`, and the inherited message says "SetInputText" even when called for a viewer ([command_executor.rs:462](../src/command_executor.rs#L462)), so the logs mislead on failure.
- **Recommendation:** pair with F-02-012. Introduce an `err` module with constructors like `err::unknown_control(window_id, control_id, op: &str) -> PlatformError` and `err::win32(op: &str, cause: windows::core::Error) -> PlatformError`. Migrate call sites mechanically. Once the stringly layer is gone, widen `PlatformError` variants to carry `{ window_id, control_id, op }` structured fields (behind `#[non_exhaustive]` so further fields are additive). The rewrite incidentally fixes the mislabelled viewer message.

### F-04-004: `Color` does not implement `Copy`, forcing `.clone()` inside paint loops
- **Severity:** Minor
- **Dimension:** quality
- **Status:** open
- **Location:** [src/styling_primitives.rs:4-12](../src/styling_primitives.rs#L4), [src/controls/listbox_handler.rs:141-167](../src/controls/listbox_handler.rs#L141), [src/controls/listbox_handler.rs:787-821](../src/controls/listbox_handler.rs#L787)
- **Observation:** `Color { r: u8, g: u8, b: u8 }` derives `Clone` but not `Copy`, even though all fields are `Copy`. Every paint path that needs to pass a color by value writes `.clone()` — the listbox row-paint picks palette colors with `.clone()` branches that would be free copies. `ListBoxState` palette setters also write `.clone()` on the way in. The crate-internal `ColorPair` in listbox_handler has the same shape.
- **Why it matters:** adding `Copy` to a public struct of three `u8`s is a fully SemVer-compatible, one-line change that removes nine `.clone()` calls in the hot paint paths and prevents a reviewer from wondering "is this `.clone()` load-bearing?" at every site. `FontWeight` (unit-variant enum) has the same shape.
- **Recommendation:** derive `Copy` on `Color` and `FontWeight` in [styling_primitives.rs](../src/styling_primitives.rs), then delete every `.clone()` the compiler reports unused. Document the add-`Copy` decision in CHANGELOG as an explicitly SemVer-compatible change.

### F-04-005: `execute_platform_command` is a ~650-line 60-variant match
- **Severity:** Major
- **Dimension:** quality
- **Status:** open
- **Location:** [src/app.rs:425-1077](../src/app.rs#L425)
- **Observation:** `Win32ApiInternalState::execute_platform_command` spans from line 425 to line ~1077 (the next method starts at line 1078), so the function is roughly 650 lines and dispatches every `PlatformCommand` variant in a single `match`. Most arms simply destructure the variant and forward the fields to a `command_executor::execute_*` function; a handful do non-trivial work (style application, dialog routing) inline. The clippy `too_many_lines` warning fires, and the file size is dominated by this function.
- **Why it matters:** adding a new command is a three-file edit (variant in `types.rs`, arm in `app.rs`, function in `command_executor.rs`); two-thirds of the labour is mechanical forwarding in `app.rs`. Reviewers scanning `app.rs` scroll past ~30% of the file before they get to the non-dispatch logic. Beyond volume, the file mixes lifecycle (`PlatformInterface` impl), command dispatch (this match), and style parsing — the same seam flagged in Phase 1
  [F-01-009](Review.01-Architecture.md#f-01-009-apprs-mixes-platforminterface-lifecycle-with-command-dispatch-and-style-parsing).
- **Recommendation:** (a) move `execute_platform_command` into `command_executor` as `command_executor::dispatch(state, command)`; (b) group dispatch arms by subsystem — layout / control-lifecycle / text / styling / dialogs / menus — with each group forwarding to a small `fn dispatch_layout(state, cmd)` returning `PlatformResult<()>`. The cleanest factoring is a `CommandKind` classifier that fans out by subsystem. See the Phase 1 refactor targets for the companion `app.rs` slim-down.

### F-04-006: `#[allow(dead_code)]` on four `pub` enums is meaningless and misleading
- **Severity:** Minor
- **Dimension:** quality
- **Status:** open
- **Location:** [src/types.rs:192](../src/types.rs#L192), [src/types.rs:366](../src/types.rs#L366), [src/types.rs:381](../src/types.rs#L381), [src/types.rs:511](../src/types.rs#L511)
- **Observation:** `#[allow(dead_code)]` annotates `DockStyle`, `MessageSeverity`, `LabelClass`, and `PlatformCommand`. The `dead_code` lint does not fire on `pub` items in a library crate, so the attribute is inert. It does, however, carry signalling weight — a reader concludes "this type has unused variants the author wants the compiler to ignore", which is not actually true of these types (they each have in-crate use sites or are reachable from consumers).
- **Why it matters:** readers rely on attributes being meaningful. A stale `#[allow]` erodes that trust and hides the question the attribute was originally answering. `PlatformCommand` especially shouldn't wear this — the crate is months away from a crates.io release and the docs-team sweep will flag it.
- **Recommendation:** remove all four `#[allow(dead_code)]`. If a warning then fires on a specific variant (unlikely — these are all `pub`), address the variant rather than reinstating the attribute.

### F-04-007: 170 `redundant_pub_crate` warnings add noise without structural value
- **Severity:** Nit
- **Dimension:** quality
- **Status:** open
- **Location:** whole tree; densest in `src/window_common.rs` and `src/controls/dialog_handler.rs`
- **Observation:** `pub(crate) fn`/`pub(crate) struct` inside `mod` items that are already private to the crate generates redundant-pub-crate warnings. They are harmless but dominate the pedantic output (170/929 warnings) and make scanning the real findings harder.
- **Why it matters:** pure noise that buries signal. At the same time, a crate-wide `#[allow(clippy::redundant_pub_crate)]` in `lib.rs` isn't obviously the right call — Phase 1 already flagged that several types were only reachable via internal paths (Phase 2 [F-02-006](Review.02-ApiAndRelease.md#f-02-006-several-public-types-are-only-reachable-via-internal-module-paths)), and keeping `pub(crate)` on the in-crate surface keeps that discipline visible.
- **Recommendation:** silence at crate level in [src/lib.rs](../src/lib.rs): `#![allow(clippy::redundant_pub_crate)]` with a one-line comment explaining that the crate prefers explicit `pub(crate)` for in-crate-public items. Leaves the pedantic pass usable.

### F-04-008: Cast-family lints hide a handful of real truncation sites
- **Severity:** Minor
- **Dimension:** quality
- **Status:** open
- **Location:** [src/controls/treeview_handler.rs:131](../src/controls/treeview_handler.rs#L131), [src/controls/treeview_handler.rs:509](../src/controls/treeview_handler.rs#L509), [src/controls/listbox_handler.rs:446](../src/controls/listbox_handler.rs#L446), [src/controls/listbox_handler.rs:475](../src/controls/listbox_handler.rs#L475); ~230 further instances across the tree (most benign)
- **Observation:** Clippy's `cast_possible_truncation` / `cast_possible_wrap` / `cast_sign_loss` / `cast_lossless` family fires on the order of 230 times. The vast majority are Win32 API coercions (`i32` ↔ `u32` ↔ `usize` at the Win32 boundary) where the types are dictated by the Win32 ABI and the value ranges are OS-constrained. A handful are genuinely risky, all sitting at `usize → i32` conversions feeding Win32 counters: `text_buffer.len() as i32` feeding `TVITEMEXW::cchTextMax` in `treeview_handler` (two sites), and `state.items.len().saturating_sub(visible) as i32` / `state.scroll_row as i32` feeding `SCROLLINFO::nMax` / `nPos` in `listbox_handler`. A pathological list or tree item would truncate silently to `i32::MAX`, then wrap.
- **Why it matters:** buried-in-noise truncation. Blanket-silencing the whole cast family at crate level masks the real finds; leaving them on makes future `cargo clippy -W pedantic` passes useless. The cited sites need explicit `i32::try_from(len).unwrap_or(i32::MAX)` or equivalent — a contract assertion rather than silent loss.
- **Recommendation:** (a) leave the cast family *enabled* at pedantic level; (b) fix the cited production truncation sites explicitly with `try_from` (or `saturating` conversion where that is the intended semantics); (c) introduce a small `win32_cast` module with named helpers (`i32_from_usize_saturating`, `u32_from_i32_unsigned`) so Win32-boundary casts are annotated at the call site and don't need per-site `#[allow]`. Do not blanket-silence.

### F-04-009: `needless_pass_by_value` fires on 22 descriptor parameters
- **Severity:** Nit
- **Dimension:** quality
- **Status:** open
- **Location:** across the control handlers and dialog handler
- **Observation:** Functions that take owned descriptor structs (`FormDialogDescriptor`, `ListBoxItemDescriptor`, `Vec<TreeItemDescriptor>`) but never move out or consume them get flagged. A handful are correctly owned (populate_treeview *does* move into the handler); most are pure read paths.
- **Why it matters:** each needless move triggers an unnecessary `clone()` upstream at the call site and masks whether the callee actually consumes the input. For a library crate, the signature communicates intent to external callers (the ones in `examples/` once Phase 2 lands).
- **Recommendation:** triage: where the callee moves fields, keep the owned param; where it only reads, change to `&`. This is mechanical but should land in one pass so reviewers can trust the signatures going forward.

### F-04-010: `Win32ApiInternalState` helpers hold the `active_windows` `RwLock` across whole function bodies
- **Severity:** Minor
- **Dimension:** quality
- **Status:** open
- **Location:** [src/app.rs:95-119](../src/app.rs#L95), [src/app.rs:320-332](../src/app.rs#L320), [src/app.rs:343-355](../src/app.rs#L343), [src/app.rs:390-418](../src/app.rs#L390)
- **Observation:** `clippy::significant_drop_tightening` flags four helper functions on `Win32ApiInternalState` that take the `active_windows` `RwLock` at the top of the body and hold it across the whole body. The body does non-lock work while the guard is live — log formatting, `describe_hwnd` building the diagnostic string, `prepare_new_window` generating the window ID and inserting into the map. None of these helpers issue `CreateWindowExW` inside the guarded scope (that happens later in `PlatformInterface::create_window`), but the lint correctly identifies that the guards outlive the work that actually needs them.
- **Why it matters:** tightening the scope is the kind of edit that's cheap today and expensive later — under a single-window lifecycle the current shape is invisible, but once `with_window_data_*` closures start wrapping control-handler bodies (they already do, transitively, for many Phase 1 finds), every extra statement under the guard becomes a sequencing point for every other in-flight command. The message loop is single-threaded, so the `RwLock` is belt-and-braces for correctness rather than a live contention source, but the `describe_hwnd` case is an obvious one — the diagnostic string is built while the lock is held, for a log line that could be assembled afterwards.
- **Recommendation:** migrate each flagged function to the pattern `{ let snapshot = with_window_data_read(|wd| …)?; … use snapshot after guard drop … }` — extract what is needed and then continue without the lock held. For `prepare_new_window`, keep the write-guard scope to the `HashMap::insert`. For `describe_hwnd`, collect the raw fields under the read guard and format the string after it drops.

### F-04-011: `self: &Arc<Self>` on methods that don't use `self`
- **Severity:** Nit
- **Dimension:** quality
- **Status:** open
- **Location:** [src/window_common.rs:2445](../src/window_common.rs#L2445), [src/window_common.rs:2593](../src/window_common.rs#L2593), [src/window_common.rs:2624](../src/window_common.rs#L2624), [src/window_common.rs:2648](../src/window_common.rs#L2648), [src/window_common.rs:2672](../src/window_common.rs#L2672), [src/window_common.rs:2702](../src/window_common.rs#L2702), [src/window_common.rs:2971](../src/window_common.rs#L2971)
- **Observation:** Seven methods on `Win32ApiInternalState` take `self: &Arc<Self>` but never read or clone `self` in the body. Clippy flags them with `unused_self`. They are stateless helpers reached through the `Arc` only because the surrounding method chain happens to have `self` in scope.
- **Why it matters:** these methods give `Win32ApiInternalState` false surface area — reviewers looking for "what does the god-object own?" see them and count them as coupled state. Freeing them turns them into standalone helpers that the refactor in Phase 1 [F-01-009](Review.01-Architecture.md#f-01-009-apprs-mixes-platforminterface-lifecycle-with-command-dispatch-and-style-parsing) can relocate without touching `Win32ApiInternalState`.
- **Recommendation:** convert each to a free function in the nearest relevant module (`window_common::layout`, `window_common::hittest`, etc.), then remove the `self: &Arc<Self>` receiver. Mechanical; land before the Phase 1 god-object split to simplify that refactor.

## API design questions (lower-confidence)

The two items below are design-level discussion topics rather than
review-grade defects. No concrete bug or usability incident supports
them today; they are captured here so the pre-1.0 API review has a
known list of shapes to re-examine. They are mirrored in the backlog
with the same `open` status as ordinary findings, but readers should
weight them accordingly.

### F-04-012: `PlatformEventHandler` couples event-in and command-out in one trait
- **Severity:** Minor
- **Dimension:** quality
- **Status:** open (API design question, not a defect)
- **Location:** [src/types.rs:859-868](../src/types.rs#L859)
- **Observation:** `PlatformEventHandler` requires both `handle_event(&mut self, event: AppEvent)` (event sink) and `try_dequeue_command(&mut self) -> Option<PlatformCommand>` (command source). The two directions are logically independent. No concrete host pain point has been reported; the coupling is a shape question, not a bug.
- **Why it matters:** pre-1.0, a split into `HandleEvent` + `CommandSource` is free; post-1.0 it is a breaking change. Worth an explicit decision before the first crates.io release rather than inheriting the current shape by default.
- **Recommendation:** treat as an open API question for the Phase 6 release-readiness pass. If the split is accepted, the migration path is a blanket `impl<T: HandleEvent + CommandSource> PlatformEventHandler for T`. If rejected, document the rationale so the question doesn't resurface.

### F-04-013: `UiStateProvider` name is broader than its two-method contract
- **Severity:** Nit
- **Dimension:** quality
- **Status:** open (API naming question, not a defect)
- **Location:** [src/types.rs:876-887](../src/types.rs#L876)
- **Observation:** `UiStateProvider` currently has exactly two methods, both specific to the TreeView custom-draw path (`is_tree_item_new`, `tree_item_marker`). The name reads as a general UI-state oracle. No concrete misuse has occurred; the concern is future-accretion — a broader name invites unrelated methods to land here.
- **Why it matters:** renaming pre-1.0 is free; post-1.0 it is a breaking change. Align with the Phase 6 naming pass.
- **Recommendation:** consider `TreeViewRenderHost` (or similar) during the Phase 6 release-readiness pass. Not urgent unless a second custom-draw consumer appears first.

## What was checked and cleared

Items inspected that produced no finding in this phase:

- **Silent fallbacks in control handlers.** Spot-checked the
  `let _ = unsafe { SendMessageW(…) }` and `let _ = … PostMessageW(…)`
  sites in `command_executor` and `window_common`. Each is covered by
  a surrounding log line or explicit `GetLastError` path; no silent
  error swallowing.
- **`unwrap()` / `expect()` on happy paths.** The surviving non-test
  sites (layout HWND unwraps at `window_common.rs:766`/`804`/`860`/`898`)
  are guarded by immediate `None`-checks; flagged in Phase 3 and
  accepted.
- **Trait object safety.** `PlatformEventHandler` and
  `UiStateProvider` are both object-safe and used behind
  `Arc<Mutex<dyn …>>` ([app.rs:1325-1326](../src/app.rs#L1325)).
  No associated-type or generic-method hazards.
- **Dead code beyond the flagged `#[allow]`s.** No other
  `#[allow(dead_code)]` in the tree; `#[cfg(not(target_os = "windows"))]`
  stubs in `styling_stub.rs` are intentional and paired with
  `#[cfg(target_os = "windows")]`.

## Cross-references

- F-04-001, F-04-002 are performance refinements; fix order is: F-04-004
  first (free `.clone()` removal), then F-04-001 (brush cache), then
  F-04-002 (UTF-16 cache).
- F-04-003 is the implementation seam that unblocks Phase 2
  [F-02-012](Review.02-ApiAndRelease.md#f-02-012-platformerror-variants-discard-structured-cause-information).
  Land together.
- F-04-005, F-04-011 both feed the Phase 1 `app.rs` / god-object
  refactor ([F-01-009](Review.01-Architecture.md#f-01-009-apprs-mixes-platforminterface-lifecycle-with-command-dispatch-and-style-parsing)).
- F-04-012, F-04-013 are not defects — they are pre-1.0 API design
  questions (see "API design questions" section above). Decide before
  the first crates.io publish, fold into Phase 6.
