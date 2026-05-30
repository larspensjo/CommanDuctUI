# Roadmap: Headless text-rendering backend for CommanDuctUI

Status: Active — Phase 1 done; Phase 2a done; Phase 2b in flight — 2026-05-30
Design spec: `docs/Spec.HeadlessRenderBackend.md`
Review notes: `docs/Review.HeadlessRenderBackend.md` (design),
`docs/Review.HeadlessRenderBackend.Phase1.md` (Phase 1)

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
| 2b | Remaining controls/commands: treeview, chart, menu, styling, scroll | `wip` | §"Phase 2b (in flight)" below |
| 2c | `--headless` stdio JSON protocol | `todo` | Spec §15 (expand when reached) |
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

## Phase 2b (in flight)

**Goal (Spec §15).** Drain the headless interpreter's single unsupported arm
(`src/headless.rs`, the `=> Err(Self::unsupported_command(command))` group) down to only
commands with no meaningful logical state. The arm currently holds exactly five
workstreams of commands:

- **TreeView (8):** `CreateTreeView`, `PopulateTreeView`, `UpdateTreeItemVisualState`,
  `UpdateTreeItemText`, `SetTreeViewSelection`, `ExpandVisibleTreeItems`,
  `ExpandAllTreeItems`, `RedrawTreeItem`.
- **Chart (2):** `CreateChart`, `SetChartData`.
- **Menu (1):** `CreateMainMenu`.
- **Styling (4):** `DefineStyle`, `ApplyStyleToControl`, `SetTabBarStyle`,
  `SetToggleSwitchStyle`.
- **Scroll (1):** `SetScrollPosition`.

**Useful starting point:** the snapshot control DTOs already reserve `scroll_vertical`,
`scroll_horizontal`, and `style_id` (currently hardcoded to `0` / `None` at
`src/headless.rs` control construction). The scroll and styling workstreams mostly
*populate existing schema* rather than extend it.

Test-first per the per-phase workflow (failing test → implement → green). Checklist:

### TreeView
- [ ] Add `ControlKind::TreeView { items: Vec<TreeItemNode>, selected_item_id: Option<…> }`
      where a node mirrors `TreeItemDescriptor` logically: raw `TreeItemId`, `text`,
      `is_folder`, `state: CheckState`, `expanded: bool`, `children`, optional
      `style_override`. No rectangles.
- [ ] Interpret `CreateTreeView` (empty), `PopulateTreeView` (build hierarchy),
      `UpdateTreeItemVisualState` (set `CheckState` by `item_id`, recursive find),
      `UpdateTreeItemText`, `ExpandVisibleTreeItems` / `ExpandAllTreeItems` (set logical
      `expanded`), `RedrawTreeItem` (recognized no-op, leaves the unsupported arm).
- [ ] `SetTreeViewSelection` — **parity exception to §7's "programmatic `Set*` is silent"
      rule.** Native is *not* silent here: `set_treeview_selection` issues `TVM_SELECTITEM`
      with no suppression flag (`src/controls/treeview_handler.rs:884`), Win32 fires
      `TVN_SELCHANGED`, and that routes unconditionally to `TreeViewItemSelectionChanged`
      (`src/window_common.rs:2162`). Headless must mirror this: update selection **and**
      enqueue a follow-up `TreeViewItemSelectionChanged { item_id }`. (Suppressing it in the
      native path instead is an alternative, but that changes the production Win32 contract
      and is out of 2b scope.)
- [ ] Semantic actions (§7-style native transition + event): toggle/select a tree item →
      `TreeViewItemToggledByUser { item_id, new_state }` and
      `TreeViewItemSelectionChanged { item_id }`. Validate the item exists; missing item →
      `Err` (mirrors "Win32 would never deliver that event").
- [ ] **Hidden-state toggle suppression** (`src/controls/treeview_handler.rs:1587`):
      toggling a tree item whose state is `CheckState::Hidden` is *valid* but emits **no**
      event and leaves the state `Hidden` (native restores the hidden image lane and returns
      `Ok(None)`). Headless must reproduce this; add an explicit test.
- [ ] Tests: each command's `UiModel` effect; semantic transitions + emitted events;
      `SetTreeViewSelection` emits the follow-up selection event (parity exception);
      hidden-row toggle is event-silent; deterministic child ordering in snapshot.

### Chart
- [ ] Add `ControlKind::Chart { data: Option<ChartDataSnapshot> }`. Per the §10 serde
      boundary, do **not** derive serde on the public `ChartDataPacket`/`ChartLineData`;
      convert to a headless-owned snapshot DTO. "Chart-as-data" means capture the full
      *logical* packet, not a subset:
      - packet-level: `week_labels`, `is_loading`, `show_x_axis_labels`,
        `show_y_axis_labels`, `show_end_labels`;
      - per line: `label`, `weekly_counts`, `end_label`, `emphasis`
        (`Primary`/`Secondary`).
      - **Intentionally omitted:** per-line `color` (raw `COLORREF` `u32`) — pure visual
        fidelity, out of logical scope (Spec §2). Document the omission in the DTO.
- [ ] Interpret `CreateChart` (empty) and `SetChartData` (replace).
- [ ] Tests: create + set-data snapshot shape and determinism, asserting the logical fields
      (including `emphasis`/`end_label`) survive and `color` is absent.

### Menu
- [ ] Add a window-level menu field on `WindowSnapshot`: a `Vec<MenuNode>` mirroring
      `MenuItemConfig` — `action: Option<u32>` (raw `MenuActionId`; **optional**, because
      popup parent nodes carry `children` and no action), `text`, `children`.
- [ ] Interpret `CreateMainMenu` → store on the window.
- [ ] Semantic action: click a menu action → `MenuActionClicked { action_id }`. Validate
      that a node carrying that action id exists; clicking a popup parent (no `action`) or
      an unknown id → `Err`. No model mutation.
- [ ] Tests: menu (incl. nested popups) appears in snapshot; action-leaf click emits the
      event; unknown id and actionless-parent clicks error.

### Styling
- [ ] Backend style registry: `DefineStyle { style_id, style }` stores the `ControlStyle`
      keyed by `StyleId`.
- [ ] `ApplyStyleToControl` populates the control's reserved `style_id` snapshot field
      with the `StyleId`'s stable string (add a stable string mapping for the `StyleId`
      enum; reuse for snapshot output).
- [ ] **Open decision:** `SetTabBarStyle` / `SetToggleSwitchStyle` carry *resolved colors*,
      not a `StyleId`, and color is out of logical scope (Spec §2). Default plan: treat
      them as recognized, event-silent **no-ops** (they leave the unsupported arm but record
      no visual data), so only `ApplyStyleToControl` drives `style_id`. Revisit if a test
      needs to observe that a style was pushed.
- [ ] Tests: `DefineStyle` + `ApplyStyleToControl` sets `style_id`; the two color-push
      commands no longer error.

### Scroll
- [ ] `SetScrollPosition` populates the reserved `scroll_vertical` / `scroll_horizontal`
      fields. **Programmatic → event-silent** — consistent with native, which suppresses the
      echo for programmatic scrolls (`is_scroll_event_suppressed`,
      `src/window_common.rs:2446`).
- [ ] Semantic action `scroll(ctrl, v, h)` → set fields → `ControlScrolled { vertical_pos,
      horizontal_pos }` (the user-driven counterpart). **Control-kind scoped:** native emits
      `ControlScrolled` only from the edit-control scroll path
      (`handle_edit_control_scroll`, `src/window_common.rs:2468`), i.e. Input / RichEdit /
      viewer-text. The action must therefore restrict to those edit-family kinds and
      **reject other kinds** — crucially listboxes, whose user scroll is a *different* event
      (`ListBoxScrolled`, `src/window_common.rs:3221`). `ListBoxScrolled` (and a listbox
      scroll action) stay out of 2b.
- [ ] Tests: `SetScrollPosition` sets fields silently for any control; `scroll` action on an
      edit-family control sets fields + emits `ControlScrolled`; `scroll` on a listbox (or
      other non-edit kind) → `Err`.

### Cross-cutting / definition of done
- [ ] Each handled command is removed from the line-869 unsupported group; confirm the
      group shrinks to only commands with no meaningful logical state (target: empty, with
      the no-wildcard `match` still forcing future variants to be handled explicitly).
- [ ] Snapshot-stability tests cover the new tree/chart/menu sections and the now-populated
      scroll/style fields.
- [ ] New public semantic actions (tree toggle/select, menu click, scroll) are a minor,
      releasable API change → bump `Cargo.toml` version and `CHANGELOG.md` together
      (next is `2.6.0`).
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

## Parking lot

Ideas surfaced but intentionally deferred, so they are not lost:

- RON snapshot output for Rust-side `insta` tests (model already serde-derived).
- Harness-owned executor for fully deterministic async (Spec §8, Phase 3).
- First-class `context_tag` fields on tagless dialog commands (semver impact — Spec §11).
- Optional default-on feature to strip headless from lean release builds (Spec §10).
