# Roadmap: Headless text-rendering backend for CommanDuctUI

Status: Active — Phase 1 done; Phase 2a done; Phase 2b done; Phase 2c implemented,
review pending — 2026-05-31
Design spec: `docs/Spec.HeadlessRenderBackend.md`
Review notes: `docs/Review.HeadlessRenderBackend.md` (design),
`docs/Review.HeadlessRenderBackend.Phase1.md` (Phase 1),
`docs/Review.HeadlessRenderBackend.Phase2c.md` (Phase 2c plan)

This is the **living control document** for an iterative, review-driven build. The spec
holds the stable "what/why"; this roadmap holds phase status, the detailed checklist for
the phase in flight, and the log of what each review taught us.

## Workflow (per phase)

1. **Implement** the phase, test-first (failing test → implement → green).
2. **External review.**
3. **Update** — apply review findings.
4. **Evaluate** — refine the **Spec** (design) and this **Roadmap** (sequence/detail) from
   what we learned; record it in the Iteration Log.

Repeat until complete.

**Definition of done (every phase):** tests green; `cargo clippy --all-targets -- -D
warnings` clean; `cargo fmt` applied; external review applied; Spec/Roadmap refined; a
short `docs/EngineeringDiary.md` entry added. Only the phase in flight is planned in
detail — later phases stay one-liners until reached. If a phase grows large, spin out a
dedicated `Plan.HeadlessRenderBackend.PhaseN.md` and link it from the table.

## Phase table

| Phase | Goal | Status | Detail |
| --- | --- | --- | --- |
| 1 | In-process headless harness over a core control subset; pump; Checkpoint/wait_for; JSON snapshot; demo + e2e | `done` | §"Phase 1 (done)" below |
| 2a | `wait_until`; dialog responder + 8 dialog commands; harden `inject_raw` | `done` | §"Phase 2a (done)" below |
| 2b | Remaining controls/commands: treeview, chart, menu, styling, scroll | `done` | §"Phase 2b (done)" below |
| 2c | `--headless` stdio JSON protocol | `wip` | §"Phase 2c (wip)" below |
| 2d | External dialog-scripting protocol (drive `Show*Dialog` outcomes from shape 2) | `todo` | Deferred from 2c review finding 1; one-liner until reached |
| 3 | Fidelity-C: shared contract suite; deterministic async; broader input vocabulary | `todo` | Spec §15 (expand when reached) |

## Phase 1 (done)

Delivered the in-process `HeadlessHarness` over a core control subset: `UiModel` with
window `shown`/`closed` + per-control `enabled`; the no-wildcard command interpreter; the
event pump with the follow-up native-event queue (`SignalMainWindowUISetupComplete` →
`MainWindowUISetupComplete`); `Checkpoint` + `wait_for` + timeout; deterministic JSON
snapshots; the §7 semantic actions; `inject_raw`; the app-core split + e2e demo test.
Review fixes (`docs/Review.HeadlessRenderBackend.Phase1.md`) are committed: same-call pump
drain, uniform visible/enabled/read-only action validation, `UpdateLabelText`, radio
grouping by `group_start`, and stronger per-effect assertions. Progress / splitter /
richedit / label-update landed here too, ahead of the original plan. See
`docs/EngineeringDiary.md`.

## Phase 2a (done)

Delivered the in-process interaction surface for headless tests: `wait_until` over the
current snapshot, dialog responder scripting with ordered matching, completions for the
save/open/profile/input/exclude-patterns/form/folder dialogs, trace-only message-box
recording, and a documented `inject_raw` escape hatch. The new public API was released as
`2.5.0`.

Out of Phase 2a (per Spec §15): treeview / chart / menu / styling / scroll (→ 2b); the
`--headless` stdio protocol (→ 2c); keyboard nav; geometry.

## Phase 2b (done)

**Goal (Spec §15).** Drain the headless interpreter's former unsupported command group
down to only commands with no meaningful logical state. The group previously held exactly five
workstreams of commands, all now implemented:

- **TreeView (8):** `CreateTreeView`, `PopulateTreeView`, `UpdateTreeItemVisualState`,
  `UpdateTreeItemText`, `SetTreeViewSelection`, `ExpandVisibleTreeItems`,
  `ExpandAllTreeItems`, `RedrawTreeItem`.
- **Chart (2):** `CreateChart`, `SetChartData`.
- **Menu (1):** `CreateMainMenu`.
- **Styling (4):** `DefineStyle`, `ApplyStyleToControl`, `SetTabBarStyle`,
  `SetToggleSwitchStyle`.
- **Scroll (1):** `SetScrollPosition`.

**Useful starting point:** the snapshot control DTOs already reserve `scroll_vertical`,
`scroll_horizontal`, and `style_id`, which Phase 2b now populates. The scroll and styling workstreams mostly
*populate existing schema* rather than extend it.

Completion checklist:

### TreeView
- [x] Add `ControlKind::TreeView { items: Vec<TreeItemNode>, selected_item_id: Option<…> }`
      where a node mirrors `TreeItemDescriptor` logically: raw `TreeItemId`, `text`,
      `is_folder`, `state: CheckState`, `expanded: bool`, `children`, optional
      `style_override`. No rectangles.
- [x] Interpret `CreateTreeView` (empty), `PopulateTreeView` (build hierarchy),
      `UpdateTreeItemVisualState` (set `CheckState` by `item_id`, recursive find),
      `UpdateTreeItemText`, `ExpandVisibleTreeItems` / `ExpandAllTreeItems` (set logical
      `expanded`), `RedrawTreeItem` (recognized no-op, leaves the unsupported arm).
- [x] `SetTreeViewSelection` — **parity exception to §7's "programmatic `Set*` is silent"
      rule.** Native is *not* silent here: `set_treeview_selection` issues `TVM_SELECTITEM`
      with no suppression flag (`src/controls/treeview_handler.rs:884`), Win32 fires
      `TVN_SELCHANGED`, and that routes unconditionally to `TreeViewItemSelectionChanged`
      (`src/window_common.rs:2162`). Headless must mirror this: update selection **and**
      enqueue a follow-up `TreeViewItemSelectionChanged { item_id }`. (Suppressing it in the
      native path instead is an alternative, but that changes the production Win32 contract
      and is out of 2b scope.)
- [x] Semantic actions (§7-style native transition + event): toggle/select a tree item →
      `TreeViewItemToggledByUser { item_id, new_state }` and
      `TreeViewItemSelectionChanged { item_id }`. Validate the item exists; missing item →
      `Err` (mirrors "Win32 would never deliver that event").
- [x] **Hidden-state toggle suppression** (`src/controls/treeview_handler.rs:1587`):
      toggling a tree item whose state is `CheckState::Hidden` is *valid* but emits **no**
      event and leaves the state `Hidden` (native restores the hidden image lane and returns
      `Ok(None)`). Headless must reproduce this; add an explicit test.
- [x] Tests: each command's `UiModel` effect; semantic transitions + emitted events;
      `SetTreeViewSelection` emits the follow-up selection event (parity exception);
      hidden-row toggle is event-silent; deterministic child ordering in snapshot.

### Chart
- [x] Add `ControlKind::Chart { data: Option<ChartDataState> }`. Per the §10 serde
      boundary, do **not** derive serde on the public `ChartDataPacket`/`ChartLineData`;
      convert to a headless-owned snapshot DTO. "Chart-as-data" means capture the full
      *logical* packet, not a subset:
      - packet-level: `week_labels`, `is_loading`, `show_x_axis_labels`,
        `show_y_axis_labels`, `show_end_labels`;
      - per line: `label`, `weekly_counts`, `end_label`, `emphasis`
        (`Primary`/`Secondary`).
      - **Intentionally omitted:** per-line `color` (raw `COLORREF` `u32`) — pure visual
        fidelity, out of logical scope (Spec §2). Document the omission in the DTO.
- [x] Interpret `CreateChart` (empty) and `SetChartData` (replace).
- [x] Tests: create + set-data snapshot shape and determinism, asserting the logical fields
      (including `emphasis`/`end_label`) survive and `color` is absent.

### Menu
- [x] Add a window-level menu field on `WindowSnapshot`: a `Vec<MenuNode>` mirroring
      `MenuItemConfig` — `action: Option<u32>` (raw `MenuActionId`; **optional**, because
      popup parent nodes carry `children` and no action), `text`, `children`.
- [x] Interpret `CreateMainMenu` → store on the window.
- [x] Semantic action: click a menu action → `MenuActionClicked { action_id }`. Validate
      that a node carrying that action id exists; clicking a popup parent (no `action`) or
      an unknown id → `Err`. No model mutation.
- [x] Tests: menu (incl. nested popups) appears in snapshot; action-leaf click emits the
      event; unknown id and actionless-parent clicks error.

### Styling
- [x] Backend style registry: `DefineStyle { style_id, style }` stores the `ControlStyle`
      keyed by `StyleId`.
- [x] `ApplyStyleToControl` populates the control's reserved `style_id` snapshot field
      with the `StyleId`'s stable string (add a stable string mapping for the `StyleId`
      enum; reuse for snapshot output).
- [x] **Decision:** `SetTabBarStyle` / `SetToggleSwitchStyle` carry *resolved colors*,
      not a `StyleId`, and color is out of logical scope (Spec §2). Default plan: treat
      them as recognized, event-silent **no-ops** (they leave the unsupported arm but record
      no visual data), so only `ApplyStyleToControl` drives `style_id`. Revisit if a test
      needs to observe that a style was pushed.
- [x] Tests: `DefineStyle` + `ApplyStyleToControl` sets `style_id`; the two color-push
      commands no longer error.

### Scroll
- [x] `SetScrollPosition` populates the reserved `scroll_vertical` / `scroll_horizontal`
      fields. **Programmatic → event-silent** — consistent with native, which suppresses the
      echo for programmatic scrolls (`is_scroll_event_suppressed`,
      `src/window_common.rs:2446`).
- [x] Semantic action `scroll(ctrl, v, h)` → set fields → `ControlScrolled { vertical_pos,
      horizontal_pos }` (the user-driven counterpart). **Control-kind scoped:** native emits
      `ControlScrolled` only from the edit-control scroll path
      (`handle_edit_control_scroll`, `src/window_common.rs:2468`), i.e. Input / RichEdit /
      viewer-text. The action must therefore restrict to those edit-family kinds and
      **reject other kinds** — crucially listboxes, whose user scroll is a *different* event
      (`ListBoxScrolled`, `src/window_common.rs:3221`). `ListBoxScrolled` (and a listbox
      scroll action) stay out of 2b.
- [x] Tests: `SetScrollPosition` sets fields silently for any control; `scroll` action on an
      edit-family control sets fields + emits `ControlScrolled`; `scroll` on a listbox (or
      other non-edit kind) → `Err`.

### Cross-cutting / definition of done
- [x] Each handled command is removed from the line-869 unsupported group; confirm the
      group shrinks to only commands with no meaningful logical state (target: empty, with
      the no-wildcard `match` still forcing future variants to be handled explicitly).
- [x] Snapshot-stability tests cover the new tree/chart/menu sections and the now-populated
      scroll/style fields.
- [x] New public semantic actions (tree toggle/select, menu click, scroll) are a minor,
      releasable API change → bump `Cargo.toml` version and `CHANGELOG.md` together
      (next is `2.6.0`).
- [x] `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt` applied;
      `docs/EngineeringDiary.md` entry added; external review applied; Spec/Roadmap refined.

## Phase 2c (wip)

**Goal (Spec §15, §9).** Ship the shipped-binary `--headless` mode: a versioned JSON-lines
**stdio protocol** (delivery shape 2) that drives the existing in-process harness from a
separate process, plus the demo app `--headless` flag. This phase adds **I/O framing and
DTOs only** — no new UI behavior, no new control modeling. The in-process harness API from
Phases 1–2b is the substrate; `run_protocol` is a thin synchronous adapter over `pump` /
actions / `snapshot` / `wait_for`.

**Scope decisions from the 2c plan review (`Review.HeadlessRenderBackend.Phase2c.md`).**
- **Dialog scripting is out of 2c (finding 1).** Shape 2 exposes only actions / snapshot /
  `wait_for`; `Show*Dialog` therefore uses the default cancel/none responder outcome over the
  protocol. This is documented as a known limitation, covered by a deterministic default-cancel
  test, and the real driver-scriptable dialog protocol is deferred to **Phase 2d**.
- **`close_window` is out of 2c (finding 7).** The protocol does not expose raw events, so it has
  no top-level window-close action; this is documented out of scope alongside `inject_raw` and
  keyboard navigation. Drivers exercise close paths via in-UI button/menu actions.
- **Snapshot enum strings are a prerequisite (finding 6).** Because shape 2 freezes
  the protocol snapshot view as the external contract, the remaining externally-visible
  `Debug`-formatted enum fields must get explicit stable-name mappings (or be documented as
  intentionally stable) **before** the DTOs ship.

**Audit / starting point (2026-05-31).**
- The harness exposes the full in-process surface already (`headless.rs`): `start`, `pump`,
  every §7 semantic action plus the 2b additions (tree select/toggle, menu click, scroll),
  `wait_for`, `wait_until`, `snapshot`, `set_dialog_responder`, `inject_raw`. **No**
  `run_protocol`, stdio adapter, or envelope DTOs exist yet — this phase adds them.
- `serde` + `serde_json` are already default deps (`Cargo.toml`), so Spec §10's dependency
  step is done; only `UiModel`/snapshot DTOs serialize today and that stays.
- Markers live in `HeadlessBackend.markers: Vec<String>` and only grow; the adapter needs a
  **cursor** to emit each checkpoint once.
- The demo (`examples/hello_window.rs`) already has the app-core split (`build_app_core`),
  but its non-Windows `main()` only builds and discards the core. Phase 2c wires the real
  `--headless` arm (the §4 `match`) so the example actually runs the protocol.
- Public id types (`WindowId`/`ControlId`/`ListBoxItemId`/`TreeItemId`/`MenuActionId`) have
  public `new` constructors and `raw()` accessors — enough to round-trip raw ids across the
  process boundary without new serde derives on the id types (Spec §10).

Completion checklist:

### Snapshot stable-name prerequisite (review finding 6)
- [ ] Replace the remaining externally-visible `format!("{:?}", …)` snapshot fields with
      explicit stable-name mappings, mirroring the Phase 2b `CheckState` / `ChartLineEmphasis` /
      `StyleId` treatment: `dock_style` (`headless.rs:2737`), dialog-request `kind` (`:3222`),
      badge `style` (`:3463`), label `class` + `severity` (`:3044-3045`), listbox `density`
      (`:3091`), splitter `orientation` (`:3203`), `MessageSeverity` (`:3300`), form note
      `severity` (`:3346`), and form `validation` (`:3382`). Prefer a stable-name helper over
      `Debug`; if any field is left `Debug`-formatted, document it as intentionally stable.
- [ ] Snapshot-stability test asserting the chosen stable strings for each converted field, so a
      future enum rename can't silently change the external protocol contract.

### Protocol DTOs (serde boundary, Spec §10)
- [ ] Add request DTOs deserialized from driver → binary: `action` (with the `click` /
      `set_text` / `select_row` / `select_combo` / `select_tab` / `toggle` / `select_radio` /
      `select_tree` / `toggle_tree` / `click_menu` / `scroll` variants), `snapshot`, and
      `wait_for`. Each carries `request_id` and raw integer ids; reconstruct opaque ids via the
      public `new` constructors. **Do not** derive serde on `PlatformCommand`/`AppEvent`/the id
      types — the DTOs are headless-owned (Spec §10).
- [ ] **Two-stage request parse (review finding 3).** First deserialize a minimal envelope
      (`type` + optional `request_id`), then deserialize the full typed request. A valid-JSON but
      invalid/malformed request that still carries a usable `request_id` produces an `error`
      response correlated with `request_id: Some(id)`; only truly unparseable input (no usable id)
      uses `request_id: null`.
- [ ] Add response DTOs serialized binary → driver: `hello` (`protocol_version`), `snapshot`
      (`model`), `ok`, `error` (`message`), the asynchronous `marker` (`label`), and the terminal
      `bye`. Tag with `#[serde(tag = "type")]`; use a protocol-specific snapshot view for `model`
      that omits cumulative `markers` and `quitting`, leaving `marker` and `bye` as the protocol
      sources of truth.
      **`error` carries `request_id: Option<u64>`** (`Some` when recoverable, `null` otherwise —
      finding 3); the correlated responses (`ok` / `snapshot`) carry the request's `request_id`.
- [ ] Define `protocol_version` as a single source of truth (const) and document the envelope
      shape next to the DTOs. **Decision:** the `hello` line carries only `protocol_version`;
      drivers obtain ids by issuing a `snapshot` first (no `create_window` request — windows are
      created by `main()` before `run_protocol`, per Spec §4/§9). Flag for review whether
      `hello` should also bundle the first snapshot for convenience.
- [ ] **Dialog handling over the protocol is default-cancel only (review finding 1).** Document,
      next to the DTOs, that shape 2 has no dialog-scripting request: `Show*Dialog` uses the
      default cancel/none responder outcome. Driver-scriptable dialog outcomes are deferred to
      Phase 2d. (No `inject_raw`, no `close_window`, no `wait_until` over the protocol either —
      finding 7 / Spec §9.)

### `run_protocol` adapter (Spec §9 loop mechanics)
- [ ] Add `HeadlessHarness::run_protocol(reader: impl BufRead, writer: impl Write) ->
      PlatformResult<()>`: write `hello`, then loop reading one JSON request per line.
- [ ] Dispatch each request to the matching in-process method; map its `PlatformResult` to an
      `ok` / `error` / `snapshot` response tagged with the request's `request_id`. A validation
      `Err` (unknown/invisible control, bad row) becomes an `error` response, **not** a process
      abort.
- [ ] `wait_for` request → keep pumping until a matching marker appears at/after the adapter's
      marker cursor, or `timeout_ms` elapses; `ok` on the marker, `error` on timeout.
      **Cursor-relative semantics (review finding 4):** protocol `wait_for` must ignore markers
      already emitted before the wait began, so a reused app-defined label (`done`, `ready`) can't
      satisfy a later wait with a stale checkpoint. (This intentionally diverges from the
      in-process `wait_for`, which scans the cumulative marker list — `headless.rs:219`; document
      the divergence.) `wait_until` is **not** exposed — Rust predicate can't cross the boundary,
      Spec §9.
- [ ] **Marker flush + writer flush (review finding 2):** keep a cursor into `backend.markers`;
      after servicing each request emit a `marker` line for every newly observed checkpoint
      *before* writing that request's response, then **`writer.flush()`** the whole output group.
      Also flush after the initial `hello` line. Without flushing, a driver blocks on serialized-
      but-unemitted bytes. (Single-threaded pump ⇒ markers only advance during a request; true
      out-of-band push stays out of 2c — parking lot.)
- [ ] **Termination + `bye` ordering (review finding 5):** return cleanly on `QuitApplication`
      (`backend.quitting`) and on reader EOF. When a request triggers quit, preserve this order:
      service the request → flush newly observed markers → write the request's tagged response →
      write `{"type":"bye"}` → flush → return. `bye` is a terminal notification, never a
      replacement for the request's correlated response. On reader EOF (no triggering request),
      flush newly observed markers, emit `bye`, flush, and return.
- [ ] Malformed/un-parseable input line → `error` response (`request_id` per the two-stage parse,
      finding 3), never a panic.
- [ ] All protocol output goes to `writer` (stdout); **all `log` output and diagnostics go to
      stderr** so stdout stays clean JSON-lines (Spec §9).

### Demo app `--headless` flag (Spec §4 reference)
- [ ] Extend `examples/hello_window.rs` `main()` with the §4 runtime `match`: default = Win32
      `main_event_loop` (Windows only), `--headless` (flag/env var) = build core →
      `HeadlessHarness::new` → `create_window` → `start` → `run_protocol(stdin().lock(),
      stdout().lock())`. Keep the app-core (`build_app_core`) compiling on non-Windows; gate
      only the Win32 `run` path.
- [ ] On non-Windows the example builds a **headless-only** binary that still runs the protocol
      (replacing today's build-and-discard `main`), proving the cross-platform path (Spec §4).
- [ ] **Clean stdout in the demo (review finding 8).** In `--headless` mode the demo must write
      no diagnostics to stdout; if it initializes a logger, configure it to stderr. Document that
      downstream apps embedding `run_protocol` must do the same — the adapter keeps its own output
      clean but cannot police a host app's logger.

### Tests
- [ ] In-process `run_protocol` tests driving it over in-memory `Cursor`/byte buffers (no real
      process): a scripted request sequence (snapshot → action → snapshot → wait_for) yields the
      expected tagged JSON-line responses, deterministically.
- [ ] Round-trip parity: a protocol `action` produces the same `UiModel` mutation + event as the
      equivalent direct harness call (assert via the resulting snapshot).
- [ ] Error paths: unknown id → `error` response (loop continues); malformed line → `error`;
      `wait_for` timeout → `error`; `QuitApplication` → `bye` + clean return.
- [ ] **Request-id recovery (finding 3):** a valid-JSON request with a bad variant/fields but a
      usable `request_id` yields `error` with `request_id: Some(id)`; truly unparseable input
      yields `request_id: null`.
- [ ] Marker ordering: a checkpoint emitted during a request appears as a `marker` line before
      that request's response, exactly once.
- [ ] **Repeated-label `wait_for` (finding 4):** after a marker `done` is observed/emitted, a
      later `wait_for("done")` blocks until a *new* `done` marker (cursor-relative), not matching
      the stale one.
- [ ] **Writer flush (finding 2):** a fake/instrumented writer proves `hello` and each request's
      output group are flushed; the test would fail if flushing were dropped.
- [ ] **`bye` ordering (finding 5):** a quit-triggering request emits its tagged response *then*
      `bye` (in that order), and the loop returns.
- [ ] **Default-cancel dialogs (finding 1):** a scripted sequence reaching a `Show*Dialog` over
      the protocol deterministically produces the default cancel/none completion (no driver
      scripting), documenting the 2c limitation.
- [ ] **Clean stdout (finding 8):** the demo `--headless` path emits only protocol JSON-lines on
      stdout (no stray diagnostics).
- [ ] `protocol_version` handshake is emitted first and is stable.

### Cross-cutting / definition of done
- [ ] New public `run_protocol` + the protocol envelope are a minor, releasable API change →
      bump `Cargo.toml` version and `CHANGELOG.md` together (next is `2.7.0`).
- [ ] `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt` applied;
      `docs/EngineeringDiary.md` entry added; external review applied; Spec/Roadmap refined.

## Iteration log

Newest entries at the bottom. One entry per review cycle: findings → resulting Spec /
Roadmap deltas.

- 2026-05-30 — Roadmap created. Spec already revised once from
  `docs/Review.HeadlessRenderBackend.md` (semantic-action state transitions, follow-up
  event pump for `SignalMainWindowUISetupComplete`, dialog responder matching, serde
  boundary, runtime-selection platform wording, `Checkpoint` rename, parity criteria).
- 2026-05-30 — Phase 1 implemented: cross-platform headless harness, deterministic JSON
  snapshots, setup-complete follow-up delivery, semantic actions, and the demo headless
  integration test landed together with the `Checkpoint` command and app-core split.
- 2026-05-30 — Phase 1 review follow-up: fixed the same-call pump regression so handler
  reaction commands drain before the action returns, aligned semantic-action validation
  with visible/enabled/read-only contracts, added label updates and radio-group
  scoping, and strengthened state assertions in the headless tests.
- 2026-05-30 — Phase 2 prep / evaluate. Confirmed Phase 1 over-delivered (progress,
  splitter, richedit, `UpdateLabelText` already done), so the remaining unsupported arm is
  treeview, dialogs, chart, menu, styling, and scroll. Split Phase 2 into 2a (interaction:
  `wait_until` + dialogs + `inject_raw`), 2b (remaining controls/commands), 2c (stdio
  protocol) to keep review cycles small. Spec refined: §9 gained the request-action schema
  and the `wait_until` in-process boundary; §11 gained responder installation/outcomes and
  the `ShowMessageBox` no-event case; §15 re-scoped into 2a/2b/2c and Phase 1 marked
  delivered.
- 2026-05-30 — Phase 2a implemented: added `wait_until`, dialog responder scripting, the
  eight modal dialog completions, dialog-request snapshot tracing, and a hardened
  `inject_raw`; bumped the crate to `2.5.0` and recorded the release in `CHANGELOG.md`.
- 2026-05-30 — Phase 2a review follow-up: replaced debug-string dialog request details
  with structured snapshot DTOs, made the `serde_json::Value` wait predicate an explicit
  public API decision in the spec, and recorded modal completion ordering as a known
  fidelity gap for later contract work.
- 2026-05-30 — Phase 2b prep / planning. Audited the headless interpreter's single
  unsupported arm and confirmed it holds exactly the five Spec §15 workstreams (treeview
  ×8, chart ×2, menu ×1, styling ×4, scroll ×1). Noted that the snapshot DTOs already
  reserve `scroll_vertical` / `scroll_horizontal` / `style_id`, so scroll and styling
  mostly populate existing schema. Expanded the in-flight 2b checklist with per-workstream
  model/command/action/test items, flagged `SetTabBarStyle` / `SetToggleSwitchStyle` as a
  no-op-vs-record open decision (color is out of logical scope), and marked 2b `wip`.
- 2026-05-30 — Phase 2b plan review (5 findings, all applied). Verified each against the
  Win32 handlers: (1) widened the chart DTO to the full logical packet
  (`show_y_axis_labels` / `show_end_labels`, per-line `end_label` / `emphasis`), omitting
  only raw `color`; (2) added the `CheckState::Hidden` toggle-suppression contract
  (`treeview_handler.rs:1587`) as a bullet + test; (3) **corrected** `SetTreeViewSelection`
  — native fires `TVN_SELCHANGED` for programmatic `TVM_SELECTITEM`
  (`treeview_handler.rs:884` → `window_common.rs:2162`), so headless must emit
  `TreeViewItemSelectionChanged`; recorded this as an explicit exception to §7 in the Spec;
  (4) made the menu node `action: Option<u32>` and required clicks to target an
  action-bearing leaf; (5) scoped the `scroll` action to edit-family controls and made it
  reject listboxes (whose user scroll is `ListBoxScrolled`, out of 2b).
- 2026-05-30 — Phase 2b implementation landed. Added headless TreeView hierarchy state,
  chart snapshots, window menu snapshots, style markers, scroll positions, tree selection
  parity, hidden-row toggle suppression, menu-action dispatch, and the public harness
  actions for tree selection/toggle, menu clicks, and semantic scrolling. Updated the
  release metadata to `2.6.0` and added snapshot/regression coverage for the new logical
  state.
- 2026-05-30 — Phase 2b review follow-up applied. Replaced new Debug-derived snapshot
  strings with explicit stable names for `CheckState` and `ChartLineEmphasis`, renamed the
  chart state/DTO types to avoid double-snapshot naming, reused the TreeView selection
  event returned by the backend, documented the retained style registry, and closed the
  Phase 2b checklist/status.
- 2026-05-31 — Phase 2c prep / planning. Audited `headless.rs`: the in-process harness
  surface (actions, `wait_for`/`wait_until`, `snapshot`, dialog responder, `inject_raw`) is
  complete, but no `run_protocol`/stdio adapter/envelope DTOs exist yet, so 2c is pure I/O
  framing over the existing pump. Confirmed `serde`/`serde_json` are already default deps
  (Spec §10 dependency step done), markers are an append-only `Vec<String>` needing a flush
  cursor, and the demo already has the app-core split but discards the core in its
  non-Windows `main`. Expanded the in-flight 2c checklist into DTO / `run_protocol` /
  demo-flag / test / DoD workstreams grounded in those findings, and refined Spec §9 with the
  adapter loop mechanics (hello handshake, no `create_window` request, id reconstruction,
  marker-flush cursor, quit/EOF termination). Flagged two decisions for the external review:
  whether `hello` should bundle the first snapshot, and deferring true out-of-band marker push
  until a real async producer exists. Marked 2c `wip`; next release is `2.7.0`.
- 2026-05-31 — Phase 2c plan review (8 findings, all applied; `Review.HeadlessRenderBackend.Phase2c.md`).
  Verified each against `headless.rs`. (1, High) External protocol can't script dialog outcomes →
  **decided** to keep dialog scripting out of 2c (default-cancel only, documented + tested) and spun
  out **Phase 2d** for the driver-scriptable dialog protocol. (2, High) Added explicit
  `writer.flush()` after `hello` and each request's output group, with an instrumented-writer test.
  (3, Medium) Specified a two-stage request parse and made the `error` response carry
  `request_id: Option<u64>` (`Some` when recoverable). (4, Medium) Pinned protocol `wait_for` to
  **cursor-relative** semantics so reused labels (`done`/`ready`) can't match a stale marker —
  diverging from the cumulative in-process `wait_for` (`headless.rs:219`); added a repeated-label
  test. (5, Medium) Specified `bye` ordering (response then `bye`, both flushed) + test. (6, Medium)
  Added a snapshot stable-name **prerequisite** workstream to convert the remaining `Debug`-formatted
  externally-visible enum fields (`dock_style`, dialog `kind`, badge `style`, label `class`/
  `severity`, `density`, splitter `orientation`, `MessageSeverity`, note `severity`, form
  `validation`) before the DTOs freeze the contract. (7, Low) Documented `close_window` out of 2c
  scope alongside `inject_raw`/keyboard nav. (8, Low) Required the demo `--headless` path to keep
  stdout clean (logger → stderr) and documented the same obligation for downstream embedders.
- 2026-05-31 — Phase 2c implementation landed. Added the `run_protocol` stdio adapter with the
  versioned JSON-lines envelope, request-id recovery for malformed input, cursor-relative
  `wait_for`, marker flushing, stable-name snapshot serialization, and the demo `--headless`
  execution path. Phase remains `wip` pending external review.
- 2026-05-31 — Phase 2c implementation review follow-up applied. Collapsed repeated protocol
  error/flush/termination scaffolding, removed the unused envelope `type` field so malformed
  requests with a recoverable `request_id` stay correlated, flushed markers before EOF `bye`,
  normalized the demo headless env flag, and made the protocol snapshot omit cumulative
  `markers`/`quitting` while keeping those fields in the in-process snapshot.

## Parking lot

Ideas surfaced but intentionally deferred, so they are not lost:

- RON snapshot output for Rust-side `insta` tests (model already serde-derived).
- Harness-owned executor for fully deterministic async (Spec §8, Phase 3).
- First-class `context_tag` fields on tagless dialog commands (semver impact — Spec §11).
- Optional default-on feature to strip headless from lean release builds (Spec §10).
- Headless currently treats `ExpandVisibleTreeItems` and `ExpandAllTreeItems` identically
  by expanding the full logical tree. Native only expands visible nodes for the former;
  revisit this if Phase 3 contract tests need strict parity.
- True out-of-band marker push in the stdio protocol (a `marker` line written with no pending
  request) is deferred from Phase 2c until a real background async producer exists; the 2c
  adapter flushes markers synchronously after each request (Spec §9).
- Driver-scriptable dialog outcomes over the stdio protocol → **Phase 2d** (deferred from the 2c
  review, finding 1). Shape 2 is default-cancel only in 2c; promote 2d when a black-box workflow
  needs to drive file/profile/form dialog results.
- `close_window` / top-level window-close simulation over the stdio protocol (review finding 7):
  out of 2c scope alongside `inject_raw` and keyboard navigation. Reconsider if a host's
  native-close logic needs black-box coverage with no in-UI close affordance.
