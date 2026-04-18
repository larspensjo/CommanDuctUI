# Plan: Thorough Review of CommanDuctUI

A multi-phase review of the CommanDuctUI source tree, with an eye toward eventual
publication as an official Rust library on crates.io. The review produces findings
and recommendations only; fixes are follow-up work driven from the rolling backlog.

This is a **living document**. Check boxes off as phases and sub-tasks complete.
Revise the plan itself when findings from one phase change the scope or priorities
of later phases; record plan changes in the "Plan revisions" section at the bottom.

## Progress at a glance

- [x] Phase 0 — Survey & framing
- [x] Phase 1 — Architecture & module boundaries
- [x] Phase 2 — Public API, documentation, crates.io readiness
- [ ] Phase 3 — Correctness & safety
- [ ] Phase 4 — Code quality & idiomatic Rust
- [ ] Phase 5 — Testability & coverage
- [ ] Phase 6 — Consolidation & release-readiness

## Approach

A hybrid review: one lightweight survey pass upfront (needed to frame everything
else), then six deep-dive phases each focused on a single dimension. Within each
phase the hot files get extra attention:

- [src/window_common.rs](../src/window_common.rs) — 3723 lines
- [src/controls/treeview_handler.rs](../src/controls/treeview_handler.rs) — 2022 lines
- [src/controls/dialog_handler.rs](../src/controls/dialog_handler.rs) — 1907 lines
- [src/app.rs](../src/app.rs) — 1681 lines
- [src/command_executor.rs](../src/command_executor.rs) — 837 lines

crates.io readiness is folded into the public API phase since the concerns overlap.

## Phases

### Phase 0 — Survey & framing

Produce the reference document every later phase cites. No findings yet; just the map.

- [x] Enumerate every `pub` item re-exported from [src/lib.rs](../src/lib.rs)
- [x] Build a module dependency diagram
- [x] Classify each source file by responsibility
- [x] Catalogue every `PlatformCommand` variant with its handler site
- [x] Catalogue every `AppEvent` variant with its emission site
- [x] List existing tests and what they cover
- [x] Produce `docs/Review.00-Survey.md`

Optional aids: the `code-review-graph` MCP tools
(`build_or_update_graph_tool`, `get_architecture_overview_tool`,
`get_hub_nodes_tool`, `get_bridge_nodes_tool`) can accelerate this.

### Phase 1 — Architecture & module boundaries

- [x] Trace a representative command (e.g. `CreateButton`) end-to-end
- [x] Trace a representative event (e.g. `ButtonClicked`) end-to-end
- [x] Confirm/contradict the `input → action → reducer → state → render` flow
  documented in [Agents.md:11](../Agents.md#L11)
- [x] Audit shared mutable state: grep for `Mutex`, `RefCell`, `Arc`,
  `static mut`, `lazy_static`, `thread_local`
- [x] Review the styling platform-split
  ([styling_primitives.rs](../src/styling_primitives.rs) /
  [styling_windows.rs](../src/styling_windows.rs) /
  [styling_stub.rs](../src/styling_stub.rs))
- [x] Identify logical seams inside each hot file and sketch proposed decompositions
- [x] Produce `docs/Review.01-Architecture.md` with findings and refactor targets

### Phase 2 — Public API, documentation, crates.io readiness

- [x] Review every item re-exported from [src/lib.rs](../src/lib.rs) for
  necessity, naming, and discoverability
- [x] Run `cargo doc --no-deps --document-private-items` and audit warnings
- [x] Audit rustdoc quality: does each public item have an example?
- [x] Audit `Cargo.toml` metadata: `keywords`, `categories`, `repository`,
  `documentation`, `readme`
- [x] Confirm `LICENSE-MIT` and `LICENSE-APACHE` files exist (currently only `LICENSE`)
- [x] Review [CHANGELOG.md](../CHANGELOG.md) discipline
- [x] Review feature-flag strategy (currently none — decide if needed)
- [x] Check for an `examples/` directory; add recommendation if missing
- [x] Review CI scaffolding (or note its absence)
- [x] Identify semver tripwires in the current API
- [x] Produce `docs/Review.02-ApiAndRelease.md` with a pre-1.0 checklist

### Phase 3 — Correctness & safety

- [ ] Audit every `unsafe` block
- [ ] Verify paired lifetimes for each Win32 resource type:
  - [ ] HWND / DestroyWindow
  - [ ] HFONT / DeleteObject
  - [ ] HBRUSH / DeleteObject
  - [ ] HMENU / DestroyMenu
  - [ ] HDC acquisition / release
  - [ ] HBITMAP / DeleteObject
  - [ ] HRGN / DeleteObject
- [ ] Audit `Drop` implementations for correctness
- [ ] Thread-safety review of shared state under the message loop
- [ ] Scan `unwrap` / `expect` sites in non-infallible positions
- [ ] Review integer/handle conversions: `as i32`, `as u32`, `as usize`,
  `transmute`
- [ ] Review message-loop reentrancy (what happens if a handler posts a
  message that re-enters the reducer?)
- [ ] Produce `docs/Review.03-Correctness.md`

### Phase 4 — Code quality & idiomatic Rust

- [ ] Run `cargo clippy --all-targets -- -W clippy::pedantic -W clippy::nursery`
  and triage output
- [ ] Review error-handling consistency: `PlatformResult` propagation vs.
  silent fallback
- [ ] Identify DRY violations across control handlers
- [ ] Identify dead code
- [ ] Audit allocation hotspots in paint/message paths (e.g. `format!` inside
  `WM_PAINT`)
- [ ] Review trait usage ergonomics for `PlatformEventHandler`, `UiStateProvider`
- [ ] Produce `docs/Review.04-CodeQuality.md`

### Phase 5 — Testability & coverage

- [ ] Inventory existing tests via `cargo test`
- [ ] Map each `PlatformCommand` variant to whether its handling is tested
- [ ] Identify pure logic currently entangled with Win32 calls that could be
  extracted for reducer-style unit tests (per [Agents.md:17-19](../Agents.md#L17-L19))
- [ ] Identify surfaces that are hard to test and document why
- [ ] Recommend test additions ordered by payoff
- [ ] Produce `docs/Review.05-Testability.md`

### Phase 6 — Consolidation & release-readiness

- [ ] Re-prioritise the full backlog across all prior phases
- [ ] Flag items that block crates.io publication vs. nice-to-haves
- [ ] Identify breaking-change items that must land pre-1.0
- [ ] Produce `docs/Review.06-ReleaseReadiness.md` with the final pre-release checklist

## Output format

### File layout

```
docs/
  Review.00-Survey.md             Phase 0 reference map
  Review.01-Architecture.md       Phase 1 findings
  Review.02-ApiAndRelease.md      Phase 2 findings + pre-1.0 checklist
  Review.03-Correctness.md        Phase 3 findings
  Review.04-CodeQuality.md        Phase 4 findings
  Review.05-Testability.md        Phase 5 findings
  Review.06-ReleaseReadiness.md   Phase 6 consolidated checklist
  Review.Backlog.md               Rolling prioritised backlog
```

### Severity scheme

- **Critical** — soundness, data races, resource leaks, panics on public API
  misuse, anything that would bite a crates.io user.
- **Major** — design flaws with real downstream cost, broken/misleading docs
  on public items, missing tests on core reducers.
- **Minor** — code quality, duplication, naming, ergonomics.
- **Nit** — style preferences, bikeshed-adjacent.

### Dimension tags

Each finding carries exactly one of: `arch`, `api`, `correctness`, `quality`,
`testing`, `release`.

### Finding format

Each finding lives in its phase report:

```markdown
### F-03-004: HFONT leaked on SetControlStyle re-application
- **Severity:** Critical
- **Dimension:** correctness
- **Status:** open
- **Location:** src/controls/styling_handler.rs:L42-L58
- **Observation:** <what is wrong>
- **Why it matters:** <concrete consequence>
- **Recommendation:** <smallest fix that resolves it>
```

IDs are stable: `F-<phase>-<nnn>`. Status is one of `open`, `in-progress`,
`fixed`, `wontfix`, `deferred`.

### Rolling backlog

[Review.Backlog.md](Review.Backlog.md) is a single table:

| ID | Severity | Dimension | Title | Status | Source |
|----|----------|-----------|-------|--------|--------|

New findings append at the bottom; status flips in place as work progresses.

## Workflow per phase

1. Read relevant source + prior phase deliverables.
2. Produce the phase report with findings (IDs assigned).
3. Append new items to `Review.Backlog.md`.
4. Tick sub-task and phase checkboxes in this plan.
5. Commit: `docs: Phase N review — <dimension>`.
6. Checkpoint before starting the next phase — revise this plan if findings
   have shifted priorities or scope.

## What this plan does not do

- It does not fix anything; it produces findings + recommendations. Fixes are
  follow-up work driven from the backlog.
- It does not enforce a timeline — phases execute when ready.
- It does not pre-decide decompositions for the hot files; those land as
  Phase 1 recommendations and only get acted on after separate approval.

## Plan revisions

Record plan changes here when findings from one phase alter later phases.

- _none yet_
