# Roadmap: Headless text-rendering backend for CommanDuctUI

Status: Active — Phase 1 done; Phase 2a done; preparing Phase 2b — 2026-05-30
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
| 2b | Remaining controls/commands: treeview, chart, menu, styling, scroll | `todo` | Spec §15 (expand when reached) |
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

## Parking lot

Ideas surfaced but intentionally deferred, so they are not lost:

- RON snapshot output for Rust-side `insta` tests (model already serde-derived).
- Harness-owned executor for fully deterministic async (Spec §8, Phase 3).
- First-class `context_tag` fields on tagless dialog commands (semver impact — Spec §11).
- Optional default-on feature to strip headless from lean release builds (Spec §10).
