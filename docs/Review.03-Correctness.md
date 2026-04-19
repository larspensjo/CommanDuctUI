# Review 03 — Correctness & safety

Phase 3 findings from [Plan.ThoroughReview](Plan.ThoroughReview.md). Each
finding is recorded once here and mirrored in
[Review.Backlog.md](Review.Backlog.md).

## Audit coverage

This phase walked the crate with a correctness/safety lens:

- every `unsafe` block in the `src` tree (Win32 calls, transmutes,
  raw-pointer GWLP_USERDATA traffic, subclass chains);
- every Win32 resource type with ownership: `HWND`, `HFONT`, `HBRUSH`,
  `HMENU`, `HDC`, `HBITMAP`, `HRGN`, `HINSTANCE` (from `LoadLibraryW`);
- every `Drop` implementation in the crate;
- non-test `unwrap`/`expect` sites;
- integer and handle casts (`as i32`, `as u32`, `as usize`, `transmute`);
- message-loop reentrancy between `WndProc`, `send_event`, the dequeue
  loop, and the `RwLock`/`Mutex` shared state.

## Scorecard

What looks solid:

- `ParsedControlStyle` — `Clone` derive removed in Phase 1 (F-01-003);
  `Drop` pairs `DeleteObject` with its `HFONT`/`HBRUSH`
  ([styling_windows.rs:18-56](../src/styling_windows.rs#L18)).
- GDI paint paths (custom-draw TreeView, ChartHandler, ToggleSwitch,
  DarkBorder) use the `SelectedObject` RAII guard
  ([controls/gdi_utils.rs:16](../src/controls/gdi_utils.rs#L16)) to
  restore originals before `DeleteObject` on created pens/brushes.
- `NativeWindowData::Drop` deletes `status_bar_font` and
  `treeview_new_item_font` ([window_common.rs:1261](../src/window_common.rs#L1261)).
- `TabBarState::Drop` deletes its font handle
  ([tab_bar_handler.rs:173](../src/controls/tab_bar_handler.rs#L173)).
- `ProgrammaticScrollGuard::Drop` pops its thread-local flag
  ([window_common.rs:159](../src/window_common.rs#L159)).
- `WM_DESTROY` recovers and drops every per-control `Box<State>` that
  lives behind `GWLP_USERDATA` — listbox
  ([listbox_handler.rs:332-341](../src/controls/listbox_handler.rs#L332)),
  tab bar ([tab_bar_handler.rs:326](../src/controls/tab_bar_handler.rs#L326)),
  toggle switch ([toggle_switch_handler.rs:241](../src/controls/toggle_switch_handler.rs#L241)),
  chart ([chart_handler.rs:131](../src/controls/chart_handler.rs#L131)),
  splitter ([splitter_handler.rs:201](../src/controls/splitter_handler.rs#L201)).
  Teardown runs at `WM_DESTROY`, not `WM_NCDESTROY`; for these custom
  controls that is sufficient (no late child-cleanup messages depend on
  the state), but the choice should be explicit if any control later
  starts creating child windows that might send messages during
  teardown.
- Dequeue-then-release: `send_event` runs after every
  `with_window_data_{read,write}` guard has been released, so a host
  callback cannot deadlock on the `RwLock<HashMap<…, NativeWindowData>>`.
- `LoadLibraryW("uxtheme.dll")` is intentionally process-lifetime;
  handle leak is by design.

Where the cracks are — see findings below.

## Findings

### F-03-001: Host `handle_event` panic can unwind across the FFI boundary (UB)
- **Severity:** Critical
- **Dimension:** correctness
- **Status:** open
- **Location:** [src/app.rs:193-210](../src/app.rs#L193), [src/window_common.rs:1384](../src/window_common.rs#L1384)
- **Observation:** `send_event` is called from inside `facade_wnd_proc_router`, an `extern "system" fn`. Inside `send_event` the host's `handle_event` runs synchronously ([app.rs:203](../src/app.rs#L203)), and no `catch_unwind` wraps the WndProc (`rg catch_unwind src/` returns no matches). A panic in host code — or any panic in the surrounding Rust code — therefore unwinds through the C ABI. The mutex story is narrower than "poisoning breaks everything": the outer `application_event_handler.lock().unwrap()` ([app.rs:197](../src/app.rs#L197)) is taken and released *before* `handle_event` runs, and the inner `handler_arc.lock()` ([app.rs:202](../src/app.rs#L202)) already uses `if let Ok(_)` with a logged fallback. So a host panic will poison `handler_arc`, but subsequent events will log and return rather than panicking again — the acute bug is the first panic unwinding through the FFI boundary, not a cascading poison-amplification.
- **Why it matters:** Rust's [FFI unwinding rules](https://doc.rust-lang.org/nomicon/ffi.html#ffi-and-unwinding) make panicking out of an `extern "system"` function undefined behaviour. In practice Win32 tears the process down with a confusing crash (and the user's window classes leak). The `.unwrap()` on the outer mutex at [app.rs:197](../src/app.rs#L197) is a separate, small latent panic source — if that mutex ever *does* get poisoned (e.g. by a future panic between `register_event_handler` and the upgrade), every subsequent event panics through the FFI boundary rather than degrading gracefully like the inner path already does.
- **Recommendation:** (a) Audit every `extern "system"` entry point in the crate and wrap each body in `std::panic::catch_unwind(AssertUnwindSafe(…))`; on `Err`, log and return `DefWindowProcW(…)` (or the class-appropriate default). Current entry points include `facade_wnd_proc_router` ([window_common.rs:1384](../src/window_common.rs#L1384)), `dark_border_subclass_proc` ([dark_border.rs:63](../src/controls/dark_border.rs#L63)), and `panel_subclass_proc` ([panel_handler.rs](../src/controls/panel_handler.rs)); any additions should be covered by the same rule. (b) Downgrade the outer `application_event_handler.lock().unwrap()` in `send_event` to match the fallback shape the inner lock already uses (`Ok` → run, `Err` → log and return). No host API impact.

### F-03-002: `listbox_handler` has a latent post-create GWLP_USERDATA overwrite hazard
- **Severity:** Major
- **Dimension:** correctness
- **Status:** open
- **Location:** [src/controls/listbox_handler.rs:975-978](../src/controls/listbox_handler.rs#L975), [src/controls/listbox_handler.rs:208](../src/controls/listbox_handler.rs#L208), [src/controls/listbox_handler.rs:253](../src/controls/listbox_handler.rs#L253)
- **Observation:** `handle_create_list_box_command` unconditionally writes `Box::into_raw(ListBoxState::new())` into `GWLP_USERDATA` *after* `CreateWindowExW` returns. Any earlier `GWLP_USERDATA` value is overwritten without being reclaimed. The lazy-init helper `get_or_init_state` allocates a default state when `GWLP_USERDATA == 0`, and it is called from several on-message paths. Windows dispatches `WM_NCCREATE`/`WM_CREATE`/`WM_SIZE` **synchronously inside `CreateWindowExW`**, so any message routed through `get_or_init_state` during creation would leave behind an orphan `Box<ListBoxState>` that the subsequent `SetWindowLongPtrW` silently overwrites. Auditing the current WndProc shows no state-touching message is guaranteed to land before the post-create overwrite on every Windows build, so this is a *hazard* — a bug waiting on the right timing or a future edit — rather than a reproducibly demonstrated leak today.
- **Why it matters:** `ListBoxState` owns no GDI handles today, so even the worst-case live leak is small. But the shape is the correctness foundation that future fields (fonts, brushes, bitmaps) will rest on; any hardening of the struct will turn a latent orphan into a real resource leak. The `WM_DESTROY` cleanup ([listbox_handler.rs:332-341](../src/controls/listbox_handler.rs#L332)) only frees the *currently-installed* pointer, so an orphaned box is unreachable from teardown.
- **Recommendation:** move state allocation *before* `CreateWindowExW` via `CREATESTRUCTW::lpCreateParams` + the `WM_NCCREATE` branch, and remove the lazy fallback — `get_or_init_state` should become `get_state`, returning an `Option` (or a `NonNull` stored via `GWLP_USERDATA`) without allocating. Do **not** make the fallback `panic!`: that would reintroduce the `extern "system"` unwind risk flagged in F-03-001. The logged-warning + ignore-the-message fallback is appropriate for a can't-happen case in a WndProc.

### F-03-003: `tab_bar_handler` has the same post-create GWLP_USERDATA overwrite hazard
- **Severity:** Major
- **Dimension:** correctness
- **Status:** open
- **Location:** [src/controls/tab_bar_handler.rs:642-645](../src/controls/tab_bar_handler.rs#L642), [src/controls/tab_bar_handler.rs:184](../src/controls/tab_bar_handler.rs#L184), [src/controls/tab_bar_handler.rs:240](../src/controls/tab_bar_handler.rs#L240), [src/controls/tab_bar_handler.rs:173](../src/controls/tab_bar_handler.rs#L173)
- **Observation:** Same shape as F-03-002: `handle_create_tab_bar_command` unconditionally writes `Box::into_raw(TabBarState::new(items))` into `GWLP_USERDATA` after `CreateWindowExW`; `get_or_init_state` lazy-initialises a default state on any message that lands there first. Unlike listbox, `TabBarState::Drop` *does* delete an `HFONT`, so if the hazard ever resolves into a real orphan, the leak includes an `HFONT`. Current message handlers on the tab bar don't touch state inside the synchronous-create window (`WM_SIZE` calls only `InvalidateRect`), so today this is latent — it becomes live the moment any `WM_NCCALCSIZE`, theme-change, or custom-draw hook starts reading state during creation.
- **Why it matters:** latent `HFONT` + `Box<TabBarState>` leak class that flips from hazard to bug with any future state-touching message handler added on the create path. Fix naturally pairs with F-03-002.
- **Recommendation:** same as F-03-002 — eager-init via `WM_NCCREATE` + `CREATESTRUCTW::lpCreateParams`, replace the lazy-init fallback with a logged "state missing" no-op. No panics from the WndProc path.

### F-03-004: Menu creation leaks `HMENU`s on two separate error paths
- **Severity:** Minor
- **Dimension:** correctness
- **Status:** open
- **Location:** [src/controls/menu_handler.rs:44-82](../src/controls/menu_handler.rs#L44), [src/controls/menu_handler.rs:90-127](../src/controls/menu_handler.rs#L90)
- **Observation:** Two distinct leak shapes live in this file.

  *Root-menu leak.* `CreateMenu()` returns `h_main_menu` at line 44. The next block calls `add_menu_item_recursive_impl` inside a `for` loop under a `with_window_data_write` closure; if that fails, the closure returns `Err(…)` which propagates out via `?` at line 61 without ever calling `DestroyMenu(h_main_menu)`. `DestroyMenu` is only invoked on the *`SetMenu` failure* branch (line 66).

  *Unattached-popup leak inside the recursive helper.* `add_menu_item_recursive_impl` ([line 113](../src/controls/menu_handler.rs#L113)) calls `CreatePopupMenu()?` to build `h_submenu`, then recurses into children ([line 115](../src/controls/menu_handler.rs#L115)), then attaches the submenu via `AppendMenuW` ([line 117](../src/controls/menu_handler.rs#L117)). If a recursive child call returns `Err`, or if the final `AppendMenuW` fails before attachment, `h_submenu` is leaked. An outer `DestroyMenu` on the root cannot reclaim it because it was never attached.
- **Why it matters:** menu creation failures are rare (`CreateMenu` / `CreatePopupMenu` / `AppendMenuW` / icon-load errors all surface here), but every failure leaks at least one `HMENU` and potentially a chain of unattached popups built by the recursive helper. The ~16k user-object limit is unlikely to bite in practice; the concern is code hygiene and, more importantly, that any future fix for the outer leak would *not* close the recursive one.
- **Recommendation:** fix both shapes together:
  1. Introduce a small RAII guard (e.g. `struct OwnedHMenu(HMENU); impl Drop { DestroyMenu(self.0); }` with a `fn disarm(self) -> HMENU`) so that *every* `CreateMenu` / `CreatePopupMenu` site owns its handle until it is successfully attached (at which point ownership transfers to the parent menu).
  2. In `add_menu_item_recursive_impl`, create `h_submenu` as an `OwnedHMenu`, recurse with its inner `HMENU`, and only `disarm` it on the line that follows a successful `AppendMenuW`.
  3. In `handle_create_main_menu_command`, apply the same pattern to `h_main_menu`, disarming on successful `SetMenu`.

### F-03-005: LOWORD/HIWORD helpers on LPARAM do not sign-extend
- **Severity:** Minor
- **Dimension:** correctness
- **Status:** open
- **Location:** [src/window_common.rs:1425-1431](../src/window_common.rs#L1425)
- **Observation:** `loword_from_lparam` and `hiword_from_lparam` return `(lparam.0 & 0xFFFF) as i32` / `((lparam.0 >> 16) & 0xFFFF) as i32`. Win32 mouse messages (`WM_MOUSEMOVE`, `WM_LBUTTONDOWN`, …) encode `(x, y)` as *signed* 16-bit coordinates in LPARAM — with multi-monitor setups they can be negative. The current helpers return 0..65535 for what should be -32768..32767. Inspection of callers shows both helpers are currently used only for WM_SIZE width/height (always non-negative), so there is no live bug, but the helpers' name signals "general LOWORD/HIWORD extraction" and any mouse-path caller added later will silently misbehave on a monitor placed to the left/above the primary.
- **Why it matters:** trap door for a future-added feature. Dark Windows multi-monitor is a real shipping configuration; the failure mode is a one-pixel-off-by-65k jump and is painful to track down.
- **Recommendation:** either rename the helpers to make their unsigned contract explicit (`loword_u16_from_lparam`) and add a parallel `get_x_lparam`/`get_y_lparam` pair that cast `as i16 as i32`, or change the existing helpers to sign-extend by default (`(lparam.0 & 0xFFFF) as i16 as i32`). The Phase 1 audit is the natural place to group this with the general utility-function cleanup.

### F-03-006: `dark_border` and `panel_handler` subclasses both alias GWLP_USERDATA for the prev-wndproc
- **Severity:** Nit
- **Dimension:** correctness
- **Status:** open
- **Location:** [src/controls/dark_border.rs:63-84](../src/controls/dark_border.rs#L63), [src/controls/panel_handler.rs:80-110](../src/controls/panel_handler.rs#L80)
- **Observation:** Both subclass installers save the original `WNDPROC` into `GWLP_USERDATA` via `SetWindowLongPtrW(hwnd, GWLP_USERDATA, prev_proc as isize)` and recover it in their subclass procs via `transmute`. This is fine per-control, but the two subclasses collide if ever installed on the same HWND: the second install would overwrite the first's prev-proc pointer, leaving a dangling WNDPROC and breaking the chain. Today dark_border is applied only to combobox/progress-bar HWNDs and panel_handler only to `Static` class panels, so no HWND receives both — but the convention is fragile and not called out anywhere in code or comment.
- **Why it matters:** defence in depth. A future "dark-border on a panel" request will compile, run, and produce a subtly broken WndProc chain. Both installers should be using `SetWindowSubclass`/`DefSubclassProc` from `comctl32` (the Win32-recommended way to stack subclasses) which carries its own per-subclass storage slot, or at minimum use distinct `GetProp`/`SetProp` string keys.
- **Recommendation:** migrate both subclasses to `SetWindowSubclass`/`RemoveWindowSubclass` (already linked against `comctl32` for TreeView). As a minimal intermediate: add a debug-assert in each installer that `GetWindowLongPtrW(hwnd, GWLP_USERDATA) == 0` before storing, so a collision panics loudly in debug builds instead of corrupting silently.

## What was checked and cleared

Items inspected that produced no finding in this phase (recorded here so
future passes don't redo the work):

- **HWND / DestroyWindow.** Window destruction is driven by the OS via
  `WM_NCDESTROY`; the crate does not call `DestroyWindow` directly.
  Per-control HWNDs are destroyed transitively by their parent. No
  double-destroy paths found.
- **HDC acquisition/release.** Every `GetDC`/`BeginPaint` site pairs
  with `ReleaseDC`/`EndPaint`; `HDC`s obtained from `PAINTSTRUCT` are
  closed exclusively through `EndPaint`.
- **HBITMAP / HRGN.** No `CreateCompatibleBitmap`/`CreateBitmap` calls
  in the tree. `HRGN` is only used as a hit-test input received from
  Win32 (`WM_NCCALCSIZE`) and is not owned by the crate.
- **HFONT inventory.** `CreateFontIndirectW` sites: `NativeWindowData`
  (status-bar, treeview-new-item), `TabBarState`, `ParsedControlStyle`,
  `chart_handler` local scopes. All are paired with `DeleteObject` in
  `Drop` or a local scope guard.
- **HBRUSH inventory.** `ParsedControlStyle::background_brush` (Drop),
  custom-draw paint scopes (`CreateSolidBrush` → `FillRect` →
  `DeleteObject` in the same block). No per-window long-lived
  `HBRUSH` fields.
- **Message-loop reentrancy (happy path).** `send_event` is always
  called *after* `with_window_data_{read,write}` guards are released
  (see e.g. [window_common.rs:2030-2052](../src/window_common.rs#L2030)),
  so a host that enqueues a command from inside `handle_event` cannot
  deadlock on the window-data lock. The remaining reentrancy question —
  "what if the host's `handle_event` opens a modal dialog that pumps
  messages?" — is the contract already flagged in Phase 1 F-01-002.
- **Non-test `.unwrap()` / `.expect()` in WndProc-reachable code.**
  The surviving hot sites are:
  - [app.rs:197](../src/app.rs#L197) — `send_event` mutex — see F-03-001.
  - [window_common.rs:766/804/860/898](../src/window_common.rs#L766) —
    layout HWND unwraps inside `layout_*` helpers; each is dominated by
    a `None` check two lines above, so the unwrap is infallible by
    construction. Acceptable but brittle; a future edit to the
    pre-check could break it silently. Considered for Phase 4 cleanup.
- **Transmute sites.** Every `std::mem::transmute` in the tree is one
  of two patterns: `GetProcAddress` → typed fn-pointer (uxtheme
  ordinals, `SetWindowCompositionAttribute`, `ShouldAppsUseDarkMode`);
  or WNDPROC restoration from GWLP_USERDATA (F-03-006). Both are
  standard Win32 interop shape.
- **`CoInitializeEx` / `CoUninitialize` pairing.** `Win32ApiInternalState::Drop`
  ([app.rs:1245](../src/app.rs#L1245)) calls `CoUninitialize` once;
  `CoInitializeEx` runs once in `new`. Single-apartment, single
  lifetime — no leak, no double-uninit.
