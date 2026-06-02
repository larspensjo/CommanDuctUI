# Roadmap: Headless text-rendering backend for CommanDuctUI

Status: Active — Phase 1 done; Phase 2a done; Phase 2b done; Phase 2c done;
Phase 2d done; Phase 3a (shared contract suite) done — 2026-06-02
Design spec: `docs/Spec.HeadlessRenderBackend.md`

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
| 2c | `--headless` stdio JSON protocol | `done` | §"Phase 2c (done)" below |
| 2d | External dialog-scripting protocol (drive `Show*Dialog` outcomes from shape 2) | `done` | §"Phase 2d (done)" below |
| 3a | Fidelity-C: shared contract-test suite (catalog + dedup-where-pure + parity tables + divergence register) | `done` | §"Phase 3a (done)" below |
| 3b | Fidelity-C: optional harness-owned executor for deterministic async | `todo` | Spec §15 (expand when reached) |
| 3c | Fidelity-C: broader input vocabulary (keyboard nav, listbox scroll) | `todo` | Spec §15 (expand when reached) |

## Phase 1 (done)

Delivered the in-process `HeadlessHarness` over a core control subset: `UiModel` with
window `shown`/`closed` + per-control `enabled`; the no-wildcard command interpreter; the
event pump with the follow-up native-event queue (`SignalMainWindowUISetupComplete` →
`MainWindowUISetupComplete`); `Checkpoint` + `wait_for` + timeout; deterministic JSON
snapshots; the §7 semantic actions; `inject_raw`; the app-core split + e2e demo test.
Review fixes are committed: same-call pump
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

## Phase 2c (done)

**Delivered.** The shipped-binary `--headless` mode landed: a versioned JSON-lines stdio
protocol (`run_protocol`) over the existing in-process pump, the demo `--headless` execution
path, and the snapshot stable-name prerequisite. Implementation and the review follow-up are
recorded in the Iteration Log (2026-05-31). The full delivered checklist is retained below for
traceability; the externally-frozen contract is the protocol snapshot view (no cumulative
`markers`/`quitting`), with markers and quit delivered through `marker`/`bye` lines.

**Goal (Spec §15, §9).** Ship the shipped-binary `--headless` mode: a versioned JSON-lines
**stdio protocol** (delivery shape 2) that drives the existing in-process harness from a
separate process, plus the demo app `--headless` flag. This phase adds **I/O framing and
DTOs only** — no new UI behavior, no new control modeling. The in-process harness API from
Phases 1–2b is the substrate; `run_protocol` is a thin synchronous adapter over `pump` /
actions / `snapshot` / `wait_for`.

**Scope decisions from the 2c plan review.**
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
- [x] Replace the remaining externally-visible `format!("{:?}", …)` snapshot fields with
      explicit stable-name mappings, mirroring the Phase 2b `CheckState` / `ChartLineEmphasis` /
      `StyleId` treatment: `dock_style` (`headless.rs:2737`), dialog-request `kind` (`:3222`),
      badge `style` (`:3463`), label `class` + `severity` (`:3044-3045`), listbox `density`
      (`:3091`), splitter `orientation` (`:3203`), `MessageSeverity` (`:3300`), form note
      `severity` (`:3346`), and form `validation` (`:3382`). Prefer a stable-name helper over
      `Debug`; if any field is left `Debug`-formatted, document it as intentionally stable.
- [x] Snapshot-stability test asserting the chosen stable strings for each converted field, so a
      future enum rename can't silently change the external protocol contract.

### Protocol DTOs (serde boundary, Spec §10)
- [x] Add request DTOs deserialized from driver → binary: `action` (with the `click` /
      `set_text` / `select_row` / `select_combo` / `select_tab` / `toggle` / `select_radio` /
      `select_tree` / `toggle_tree` / `click_menu` / `scroll` variants), `snapshot`, and
      `wait_for`. Each carries `request_id` and raw integer ids; reconstruct opaque ids via the
      public `new` constructors. **Do not** derive serde on `PlatformCommand`/`AppEvent`/the id
      types — the DTOs are headless-owned (Spec §10).
- [x] **Two-stage request parse (review finding 3).** First deserialize a minimal envelope
      (`type` + optional `request_id`), then deserialize the full typed request. A valid-JSON but
      invalid/malformed request that still carries a usable `request_id` produces an `error`
      response correlated with `request_id: Some(id)`; only truly unparseable input (no usable id)
      uses `request_id: null`.
- [x] Add response DTOs serialized binary → driver: `hello` (`protocol_version`), `snapshot`
      (`model`), `ok`, `error` (`message`), the asynchronous `marker` (`label`), and the terminal
      `bye`. Tag with `#[serde(tag = "type")]`; use a protocol-specific snapshot view for `model`
      that omits cumulative `markers` and `quitting`, leaving `marker` and `bye` as the protocol
      sources of truth.
      **`error` carries `request_id: Option<u64>`** (`Some` when recoverable, `null` otherwise —
      finding 3); the correlated responses (`ok` / `snapshot`) carry the request's `request_id`.
- [x] Define `protocol_version` as a single source of truth (const) and document the envelope
      shape next to the DTOs. **Decision:** the `hello` line carries only `protocol_version`;
      drivers obtain ids by issuing a `snapshot` first (no `create_window` request — windows are
      created by `main()` before `run_protocol`, per Spec §4/§9). Flag for review whether
      `hello` should also bundle the first snapshot for convenience.
- [x] **Dialog handling over the protocol is default-cancel only (review finding 1).** Document,
      next to the DTOs, that shape 2 has no dialog-scripting request: `Show*Dialog` uses the
      default cancel/none responder outcome. Driver-scriptable dialog outcomes are deferred to
      Phase 2d. (No `inject_raw`, no `close_window`, no `wait_until` over the protocol either —
      finding 7 / Spec §9.)

### `run_protocol` adapter (Spec §9 loop mechanics)
- [x] Add `HeadlessHarness::run_protocol(reader: impl BufRead, writer: impl Write) ->
      PlatformResult<()>`: write `hello`, then loop reading one JSON request per line.
- [x] Dispatch each request to the matching in-process method; map its `PlatformResult` to an
      `ok` / `error` / `snapshot` response tagged with the request's `request_id`. A validation
      `Err` (unknown/invisible control, bad row) becomes an `error` response, **not** a process
      abort.
- [x] `wait_for` request → keep pumping until a matching marker appears at/after the adapter's
      marker cursor, or `timeout_ms` elapses; `ok` on the marker, `error` on timeout.
      **Cursor-relative semantics (review finding 4):** protocol `wait_for` must ignore markers
      already emitted before the wait began, so a reused app-defined label (`done`, `ready`) can't
      satisfy a later wait with a stale checkpoint. (This intentionally diverges from the
      in-process `wait_for`, which scans the cumulative marker list — `headless.rs:219`; document
      the divergence.) `wait_until` is **not** exposed — Rust predicate can't cross the boundary,
      Spec §9.
- [x] **Marker flush + writer flush (review finding 2):** keep a cursor into `backend.markers`;
      after servicing each request emit a `marker` line for every newly observed checkpoint
      *before* writing that request's response, then **`writer.flush()`** the whole output group.
      Also flush after the initial `hello` line. Without flushing, a driver blocks on serialized-
      but-unemitted bytes. (Single-threaded pump ⇒ markers only advance during a request; true
      out-of-band push stays out of 2c — parking lot.)
- [x] **Termination + `bye` ordering (review finding 5):** return cleanly on `QuitApplication`
      (`backend.quitting`) and on reader EOF. When a request triggers quit, preserve this order:
      service the request → flush newly observed markers → write the request's tagged response →
      write `{"type":"bye"}` → flush → return. `bye` is a terminal notification, never a
      replacement for the request's correlated response. On reader EOF (no triggering request),
      flush newly observed markers, emit `bye`, flush, and return.
- [x] Malformed/un-parseable input line → `error` response (`request_id` per the two-stage parse,
      finding 3), never a panic.
- [x] All protocol output goes to `writer` (stdout); **all `log` output and diagnostics go to
      stderr** so stdout stays clean JSON-lines (Spec §9).

### Demo app `--headless` flag (Spec §4 reference)
- [x] Extend `examples/hello_window.rs` `main()` with the §4 runtime `match`: default = Win32
      `main_event_loop` (Windows only), `--headless` (flag/env var) = build core →
      `HeadlessHarness::new` → `create_window` → `start` → `run_protocol(stdin().lock(),
      stdout().lock())`. Keep the app-core (`build_app_core`) compiling on non-Windows; gate
      only the Win32 `run` path.
- [x] On non-Windows the example builds a **headless-only** binary that still runs the protocol
      (replacing today's build-and-discard `main`), proving the cross-platform path (Spec §4).
- [x] **Clean stdout in the demo (review finding 8).** In `--headless` mode the demo must write
      no diagnostics to stdout; if it initializes a logger, configure it to stderr. Document that
      downstream apps embedding `run_protocol` must do the same — the adapter keeps its own output
      clean but cannot police a host app's logger.

### Tests
- [x] In-process `run_protocol` tests driving it over in-memory `Cursor`/byte buffers (no real
      process): a scripted request sequence (snapshot → action → snapshot → wait_for) yields the
      expected tagged JSON-line responses, deterministically.
- [x] Round-trip parity: a protocol `action` produces the same `UiModel` mutation + event as the
      equivalent direct harness call (assert via the resulting snapshot).
- [x] Error paths: unknown id → `error` response (loop continues); malformed line → `error`;
      `wait_for` timeout → `error`; `QuitApplication` → `bye` + clean return.
- [x] **Request-id recovery (finding 3):** a valid-JSON request with a bad variant/fields but a
      usable `request_id` yields `error` with `request_id: Some(id)`; truly unparseable input
      yields `request_id: null`.
- [x] Marker ordering: a checkpoint emitted during a request appears as a `marker` line before
      that request's response, exactly once.
- [x] **Repeated-label `wait_for` (finding 4):** after a marker `done` is observed/emitted, a
      later `wait_for("done")` blocks until a *new* `done` marker (cursor-relative), not matching
      the stale one.
- [x] **Writer flush (finding 2):** a fake/instrumented writer proves `hello` and each request's
      output group are flushed; the test would fail if flushing were dropped.
- [x] **`bye` ordering (finding 5):** a quit-triggering request emits its tagged response *then*
      `bye` (in that order), and the loop returns.
- [x] **Default-cancel dialogs (finding 1):** a scripted sequence reaching a `Show*Dialog` over
      the protocol deterministically produces the default cancel/none completion (no driver
      scripting), documenting the 2c limitation.
- [x] **Clean stdout (finding 8):** the demo `--headless` path emits only protocol JSON-lines on
      stdout (no stray diagnostics).
- [x] `protocol_version` handshake is emitted first and is stable.

### Cross-cutting / definition of done
- [x] New public `run_protocol` + the protocol envelope are a minor, releasable API change →
      bump `Cargo.toml` version and `CHANGELOG.md` together (next is `2.7.0`).
- [x] `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt` applied;
      `docs/EngineeringDiary.md` entry added; external review applied; Spec/Roadmap refined.

## Phase 2d (done)

**Delivered.** The protocol v2 `set_dialog_responder` request landed: headless-owned matcher/outcome
DTOs reconstruct the existing in-process `DialogScriptEntry` script (including `FormFieldValue`s)
across the stdio boundary, the dispatch arm installs the script with replace-semantics, and the
existing pump turns a matched outcome into the precise `…Completed` event. The review follow-up
unified file/folder outcomes on the external `path` field, kept `message_box` entries out of the
FIFO so they cannot block later dialogs, and documented replace/install-before-action semantics.
Released as `2.8.0`, `protocol_version` 1 → 2. The full delivered checklist is retained below for
traceability.

**Goal (Spec §9, §11, §15).** Let an external (shape 2) driver **script `Show*Dialog`
outcomes** over the stdio protocol, lifting the 2c default-cancel-only limitation. Like 2c,
this phase adds **protocol framing and DTOs only** — no new UI behavior and no new dialog
modeling. The in-process dialog responder (`set_dialog_responder` + the ordered
`DialogScriptEntry` queue, `headless.rs:506`) is the substrate; 2d exposes a request that fills
that same queue from JSON, and the existing pump (`take_dialog_outcome_for`, `headless.rs:1692`)
already turns a matched outcome into the precise `…Completed` event.

**Audit / starting point (2026-05-31).**
- In-process dialog scripting is complete: `DialogMatcher` (kind + optional
  `window_id`/`title`/`prompt`/`context_tag`, `headless.rs:66`), `DialogScriptEntry`,
  `DialogOutcome` (`headless.rs:93`: SaveFile / OpenFile / ProfileSelection / Input /
  ExcludePatterns / Form / FolderPicker / MessageBox), and the ordered match-or-default
  fallback (`take_dialog_outcome_for` → `default_dialog_outcome`, `headless.rs:1701`). The
  protocol cannot reach any of it today — `ProtocolRequest` has only `Action` / `Snapshot` /
  `WaitFor` (`headless.rs:173`).
- **Serde boundary (Spec §10):** `DialogMatcher`, `DialogScriptEntry`, `DialogOutcome`,
  `DialogKind`, and `FormFieldValue` (`types.rs:627`) are **not** serde-derived and must stay
  that way. 2d adds **headless-owned protocol DTOs** that deserialize into them, mirroring how
  `ProtocolActionRequest` carries raw integer ids and reconstructs the opaque id types.
- Dialog requests are already captured in the snapshot, so a driver can poll to confirm which
  dialog fired and whether the script consumed an entry — no new observability needed.
- `MessageBox` has no completion event and consumes no responder entry (§11); its outcome DTO is
  the no-payload `message_box` variant, kept only for symmetry/validation.

**Design decision (locked in): Approach A — a pre-scripted `set_dialog_responder` request.** A
new `{"type":"set_dialog_responder","request_id":N,"script":[{matcher, outcome}, …]}` request
installs the ordered script *ahead* of the action that opens the dialog — a thin protocol mirror
of `HeadlessHarness::set_dialog_responder`, with replace-semantics like the in-process call. It
fits the existing synchronous request→response loop with no pump changes, which is exactly what
the intended use — **deterministic integration testing** — needs: the test knows the dialog
sequence in advance, so a pre-scripted queue is sufficient and there is no need for the driver to
react to dialogs as they appear. The reactive alternative (the pump suspends on a `Show*Dialog`
and blocks for a driver `dialog_response`) is **not** pursued; it stays parked for a possible
future interactive black-box harness.

Completion checklist:

### Protocol DTOs (serde boundary, Spec §10)
- [x] Add a `set_dialog_responder` request DTO: `request_id` plus `script: Vec<DialogScriptEntryDto>`,
      where `DialogScriptEntryDto` = `{ matcher: DialogMatcherDto, outcome: DialogOutcomeDto }`.
      Deserialize into the existing `DialogScriptEntry`; **do not** derive serde on the public
      dialog types.
- [x] `DialogMatcherDto`: `kind` (stable `DialogKind` string — add the stable-name mapping if one
      doesn't exist yet, consistent with the §10 enum-string rule), optional `window_id` (raw
      `usize` → `WindowId::new`), optional `title` / `prompt` / `context_tag`.
- [x] `DialogOutcomeDto`: tagged by dialog kind, carrying the same logical payload as
      `DialogOutcome` — `save_file`/`open_file`/`folder_picker` (`path: Option<String>` → `PathBuf`),
      `profile_selection` (`chosen_profile_name`, `create_new_requested`, `user_cancelled`),
      `input` (`text: Option<String>`), `exclude_patterns` (`saved`, `patterns`), `form`
      (`confirmed` + `field_values`), `message_box` (no payload). A `matcher.kind` /
      `outcome` mismatch is a validation `error`, not a panic (mirror the in-process
      kind/outcome pairing in `apply_dialog_outcome`).
- [x] `FormFieldValueDto` reconstructing `FormFieldValue` (`types.rs:627`): `text`
      (`field_id`, `value`) and `checkbox` (`field_id`, `checked`).
- [x] Bump `protocol_version` (1 → 2) since the request vocabulary grows; document the new request
      next to the existing envelope docs, including that the script **replaces** any prior script
      and must be installed before the triggering action.

### `run_protocol` dispatch (Spec §9)
- [x] Add the `SetDialogResponder` arm to `ProtocolRequest` and dispatch it to
      `set_dialog_responder` after reconstructing the script; respond `ok` (or `error` on a
      malformed/mismatched entry). No model mutation, no pump — the script only affects subsequent
      `Show*Dialog` execution.
- [x] Confirm the existing fallback still holds over the protocol: a dialog with no matching script
      entry resolves to default cancel/none (the 2c behavior), so a missing script never hangs.
- [x] Two-stage parse + `request_id` recovery unchanged: a `set_dialog_responder` with a usable
      `request_id` but a bad script entry yields `error` with `request_id: Some(id)`.

### Demo app reference (Spec §4)
- [x] Extend the demo `--headless` reference (or the e2e protocol test) to drive at least one
      `Show*Dialog` to a scripted non-default outcome over the protocol, proving the round-trip
      and documenting it as the shape-2 dialog-scripting reference.

### Tests
- [x] `run_protocol` over in-memory `Cursor`: `set_dialog_responder` → action that opens the
      dialog → `snapshot`/`wait_for` shows the scripted `…Completed` effect (e.g. a saved path,
      a confirmed form). Parity with the equivalent in-process `set_dialog_responder` call.
- [x] Default-cancel still applies when the script is empty or no entry matches (regression guard
      on the 2c limitation now that scripting exists).
- [x] `MessageBox` outcome consumes no entry and emits no event over the protocol.
- [x] Error paths: matcher/outcome kind mismatch → `error` (loop continues); malformed script
      entry with a usable `request_id` → `error` with `request_id: Some(id)`.
- [x] `FormFieldValueDto` round-trips text + checkbox fields into the resulting
      `FormDialogCompleted` field values.
- [x] `protocol_version` is `2` and stable.

### Cross-cutting / definition of done
- [x] New `set_dialog_responder` protocol request + the dialog DTOs + `protocol_version` bump are a
      minor, releasable API change → bump `Cargo.toml` version and `CHANGELOG.md` together (next is
      `2.8.0`).
- [x] `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt` applied;
      `docs/EngineeringDiary.md` entry added; external review applied; Spec/Roadmap refined.

## Phase 3a (done)

**Goal (Spec §14, §15, §16).** Begin the road to fidelity-C with the **shared contract-test suite** —
the principal mitigation for fidelity drift (§16). This sub-phase establishes two durable structures
and seeds them from the contracts that exist today; later behaviors append to the same structures as
they land. It is **not** about new control modeling or new input vocabulary (that is 3b/3c).

The two structures (Spec §14, §16):
- **Contract catalog** — a single referenceable list of documented behaviors both backends must
  honor, each tagged *shared-pure* (extracted to one source of truth) or *parity-only* (pinned by a
  data-driven table).
- **Divergence register** — every known Win32↔headless divergence with a deliberate disposition
  under the standing **fix-or-pin rule**: fix headless to match, or accept-with-pinning-test plus a
  recorded rationale.

**Design decisions (locked in from this prep cycle).**
- **Dedup where pure; parity tables for the rest.** Genuinely-pure behaviors (no `HWND`, no
  geometry) are extracted into one shared source of truth so drift is structurally impossible;
  irreducible behaviors (Win32 routes through native messages) are pinned by data-driven tables
  asserted on headless and mirrored on Win32 behind `#[cfg(target_os = "windows")]` where feasible.
  Aggressive whole-reducer extraction is rejected (YAGNI; risk to the production Win32 path).
- **Both current divergences are dispositioned `accept` + pinning test**, for different reasons:
  modal-completion ordering is an intentional non-blocking-pump choice; `ExpandVisible`/`ExpandAll`
  hinges on viewport visibility, which is geometry and therefore out of scope (Spec §2).

**Audit / starting point (2026-06-01; implementation update 2026-06-02).**
- `validate_layout_rules` started **duplicated verbatim** between Win32 and headless. It is now
  extracted to `src/contracts.rs`, with both backends calling the shared pure seam and a table test
  covering the existing validation rules. The extraction also makes Win32's multi-parent
  layout-violation error ordering deterministic.
- Other *shared-pure* candidates to confirm during 3a: radio grouping by `group_start`
  (`src/controls/radiobutton_handler.rs`), and the disabled-row-selectable predicate.
- *Parity-only* contracts already implemented on both sides but not asserted as a shared table:
  programmatic-`Set*` silence + the `SetTreeViewSelection` event-bearing exception (Spec §7);
  hidden-state tree-toggle suppression (`src/controls/treeview_handler.rs:1587`); disabled listbox
  rows remain selectable.
- Native `expand_visible_tree_items` (`src/controls/treeview_handler.rs:722`) walks
  `TVGN_FIRSTVISIBLE`/`TVGN_NEXTVISIBLE` — confirms the expand-visible divergence is geometry-bound.

Completion checklist:

### Contract catalog
- [x] Add the contract-catalog list (Spec §14) as the index of the documented behaviors, each tagged
      *shared-pure* or *parity-only*. Seed it with the behaviors enumerated in the audit; document
      that it is append-only and load-bearing.
- [x] Decide the catalog's physical home (a `#[cfg(test)]` contract module that enumerates the
      data-driven cases, referenced from the Spec) so future behaviors have an obvious place to land.
      Decision: `src/contracts.rs` owns shared pure seams and the append-only test catalog; headless
      parity tables stay in `src/headless/tests.rs` where backend internals are reachable.

### Dedup the shared-pure seams (one source of truth)
- [x] Extract `validate_layout_rules` into a single shared pure function both backends call; delete
      the headless copy and the Win32 copy's body in favor of the shared seam. No behavior change,
      no geometry — the error strings stay identical.
- [x] Test the shared seam once (table of layout-rule inputs → expected `Ok`/`PlatformError`),
      covering the docked-edge-needs-`fixed_size`, negative-`fixed_size`, and one-`Fill`-per-parent
      rules.
- [x] Confirm the further *shared-pure* candidates (radio grouping by `group_start`,
      disabled-row-selectable predicate); extract the ones that are genuinely pure, or reclassify
      them *parity-only* in the catalog with a note on why they can't be deduplicated.
      Decision: both stay *parity-only* for now. Radio grouping is native button-group behavior on
      Win32; disabled-row selection is a user/native-event contract rather than a standalone shared
      reducer seam.

### Parity tables (irreducible behaviors)
- [x] Add data-driven parity tables for the *parity-only* contracts asserted on the headless backend:
      programmatic-`Set*` silence + the `SetTreeViewSelection` exception; hidden-state tree-toggle
      suppression; disabled-row-selectable. Each row is (input → expected logical outcome / emitted
      event-or-silence).
- [x] Mirror the same tables on Win32 behind `#[cfg(target_os = "windows")]` where a pure seam is
      reachable; where Win32 truly needs an `HWND`, document the table as headless-asserted with the
      Win32 contract referenced by code location.
      Decision: no extra Win32 mirror was added for native-message-backed contracts because they need
      HWND/control message routing. Existing Win32 pure tests now call the shared layout seam; the
      remaining rows are pinned on headless and tied to native code locations in the Spec/Roadmap.

### Divergence register
- [x] Record the divergence register (Spec §16) with both current entries dispositioned `accept`:
      modal-completion ordering (non-blocking pump) and `ExpandVisible`/`ExpandAll` (geometry-bound).
- [x] Add a pinning test for each accepted divergence that locks the documented current behavior, so
      an accidental change to either surfaces as a test failure rather than silent drift.
- [x] Document the standing **fix-or-pin rule** so every future divergence gets a disposition.

### Cross-cutting / definition of done
- [x] Determine the release impact: if the dedup is internal-only (no public API change), **no
      version bump** is required; if any public surface changes, bump `Cargo.toml` + `CHANGELOG.md`
      together. Decision: no public surface changed; no version bump and no changelog entry.
- [x] `cargo build` and `cargo test` green during implementation.
- [x] `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt` applied;
      `docs/EngineeringDiary.md` entry added; external review applied; Spec/Roadmap refined.

## Iteration log

Newest entries at the bottom. One entry per review cycle: findings → resulting Spec /
Roadmap deltas.

- 2026-05-30 — Roadmap created. Spec already revised once from the design review
  (semantic-action state transitions, follow-up event pump for
  `SignalMainWindowUISetupComplete`, dialog responder matching, serde boundary,
  runtime-selection platform wording, `Checkpoint` rename, parity criteria).
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
- 2026-05-31 — Phase 2c plan review (8 findings, all applied).
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
  `markers`/`quitting` while keeping those fields in the in-process snapshot. Phase 2c marked
  `done`.
- 2026-05-31 — Phase 2d prep / planning. Promoted dialog scripting from the parking lot to the
  in-flight phase. Audited the dialog seams in `headless.rs`: in-process scripting is complete
  (`DialogMatcher`/`DialogScriptEntry`/`DialogOutcome` + the `take_dialog_outcome_for` ordered
  fallback), but `ProtocolRequest` exposes none of it. Confirmed the public dialog types and
  `FormFieldValue` are not serde-derived, so 2d adds headless-owned DTOs that reconstruct them
  (same pattern as the 2c action DTOs). Expanded the in-flight 2d checklist into DTO / dispatch /
  demo / test / DoD workstreams grounded in those findings, and **locked in Approach A** (a
  pre-scripted `set_dialog_responder` request that fits the synchronous loop) — sufficient for the
  intended deterministic integration testing, where the dialog sequence is known in advance. The
  reactive Approach B (a `dialog_request`/`dialog_response` that suspends the pump mid-turn) stays
  parked for a possible future interactive harness. Marked 2d `wip`; next release is `2.8.0`,
  `protocol_version` bumps 1 → 2.
- 2026-06-01 — Phase 2d implemented + review follow-up applied. Added the protocol v2
  `set_dialog_responder` request with headless-owned matcher/outcome DTOs that reconstruct the
  in-process `DialogScriptEntry` script (including `FormFieldValue`s), the dispatch arm with
  replace-semantics, and the demo/e2e dialog-scripting reference. Review follow-up unified
  file/folder outcomes on the external `path` field, kept `message_box` entries out of the FIFO so
  they cannot block later dialogs, and documented replace/install-before-action semantics. Released
  as `2.8.0`, `protocol_version` 1 → 2. Subsequently split the monolithic `headless.rs` into the
  `src/headless/` module (`backend` / `protocol` / `snapshot` / `state` / `tests`); behavior
  unchanged. Marked 2d `done`.
- 2026-06-01 — Phase 3 prep / planning (toward fidelity-C). Split Phase 3 into rolling-wave
  sub-phases — 3a shared contract-test suite (in flight), 3b deterministic-async executor, 3c
  broader input vocabulary — detailing only 3a. Audited the documented behaviors: confirmed
  `validate_layout_rules` was duplicated **verbatim** across `window_common.rs:995` and
  `headless/backend.rs:922` (the canonical *shared-pure* extraction target), and that
  `expand_visible_tree_items` (`treeview_handler.rs:722`) keys on `TVGN_*VISIBLE`, confirming the
  expand-visible divergence is geometry-bound. Locked in two design decisions: (1) **dedup
  where pure, parity tables for the rest** — extract genuinely-pure seams to one source of truth
  (drift becomes impossible), pin irreducible behaviors with data-driven tables; aggressive
  whole-reducer extraction rejected (YAGNI / Win32-path risk); (2) a standing **fix-or-pin rule**
  for divergences, with both current divergences (modal-completion ordering; expand-visible/all)
  dispositioned `accept` + pinning test for distinct reasons (intentional non-blocking pump;
  geometry out of scope). Refined Spec §14 (contract catalog), §15 (Phase 3 split), and §16
  (divergence register + fix-or-pin rule). Expanded the in-flight 3a checklist into
  catalog / dedup / parity-table / divergence-register / DoD workstreams. Marked 3a `wip`; no
  release planned yet (likely internal-only, version TBD during implementation).
- 2026-06-02 — Phase 3a implementation. Added `src/contracts.rs` as the physical home for shared
  pure contracts and seeded its append-only catalog. Extracted `validate_layout_rules` so Win32 and
  headless both call the same pure function, with a table test covering the current layout
  validation rules. Added headless parity tables for programmatic `Set*` silence, the
  `SetTreeViewSelection` event-bearing exception, disabled listbox row selection, and the existing
  hidden tree-toggle suppression contract. Added pinning tests for the two accepted divergences:
  modal dialog completions run after already queued commands in headless, and
  `ExpandVisibleTreeItems` expands the full logical tree because viewport geometry is out of scope.
  No public API changed, so no version bump or changelog entry.
- 2026-06-02 — Phase 3a implementation review follow-up applied. Softened the catalog wording to
  reflect that parity-only entries are review-enforced rather than compile-time registered, named
  the backing tests in the catalog, strengthened the programmatic `Set*` table to assert both
  silence and state mutation, and recorded that sharing `validate_layout_rules` also de-randomized
  Win32's multi-parent layout-violation error ordering. Marked 3a `done`.

## Parking lot

Ideas surfaced but intentionally deferred, so they are not lost:

- RON snapshot output for Rust-side `insta` tests (model already serde-derived).
- Harness-owned executor for fully deterministic async (Spec §8) → **Phase 3b**.
- First-class `context_tag` fields on tagless dialog commands (semver impact — Spec §11).
- Optional default-on feature to strip headless from lean release builds (Spec §10).
- Headless currently treats `ExpandVisibleTreeItems` and `ExpandAllTreeItems` identically
  by expanding the full logical tree. Native only expands visible nodes for the former → moved
  into the **Phase 3a divergence register** (Spec §16), dispositioned `accept` + pinning test
  because "visible" is geometry (out of scope, Spec §2). Revisit only if a geometry-bearing need
  appears.
- True out-of-band marker push in the stdio protocol (a `marker` line written with no pending
  request) is deferred from Phase 2c until a real background async producer exists; the 2c
  adapter flushes markers synchronously after each request (Spec §9).
- Driver-scriptable dialog outcomes over the stdio protocol → **delivered in Phase 2d** as
  Approach A (the pre-scripted `set_dialog_responder` request) for deterministic integration
  testing. Approach B — a reactive `dialog_request`/`dialog_response` that suspends the pump mid-turn
  for a fully interactive black-box harness — stays parked here for a possible future need.
- `close_window` / top-level window-close simulation over the stdio protocol (review finding 7):
  out of 2c scope alongside `inject_raw` and keyboard navigation. Reconsider if a host's
  native-close logic needs black-box coverage with no in-UI close affordance.
