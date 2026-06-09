# Roadmap: Headless text-rendering backend for CommanDuctUI

Status: Active — started 2026-05-30
Design spec: `docs/Spec.HeadlessRenderBackend.md`
Review notes: `docs/Review.HeadlessRenderBackend.md`

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
| 1 | In-process headless harness over a core control subset; pump; Checkpoint/wait_for; JSON snapshot; demo + e2e | `todo` | §"Phase 1 (current)" below |
| 2 | Predicate waits; dialog responder; more controls; `--headless` stdio JSON protocol | `todo` | Spec §15 (expand when reached) |
| 3 | Fidelity-C: shared contract suite; deterministic async; broader input vocabulary | `todo` | Spec §15 (expand when reached) |

## Phase 1 (current)

Goal: a working **in-process Rust harness** that interprets a core `PlatformCommand`
subset into a `UiModel`, supports native-state-transition actions, reaches "DONE" via
synchronous quiescence + `Checkpoint`/`wait_for`, and serializes JSON — proven by a demo
app and one end-to-end test. Must meet the Spec §13 acceptance criteria.

Ordered, test-first checklist (each step lands its own tests before moving on):

- **1.1 Scaffolding & dependency boundary**
  - Add `serde` + `serde_json` as unconditional deps; add `pub mod headless` compiling on
    all platforms.
  - Add `PlatformCommand::Checkpoint { label }`; implement the Win32 executor as log-only;
    document its distinction from `SignalMainWindowUISetupComplete` in the enum docs.
  - Releasable surface (new public command): bump `Cargo.toml` version + `CHANGELOG.md`
    together at phase close (§1.8).
  - Tests: headless records `Checkpoint` (1.5); build proves the Win32 match stays
    exhaustive.

- **1.2 `UiModel` + serde DTOs**
  - Model: windows (`shown`/`closed`, title, controls) → typed control nodes (Button /
    Label / Input / ListBox / CheckBox / Radio / Toggle / Combo / TabBar) with logical
    properties + `enabled`; containment via `parent_control_id`; `DefineLayout` recorded
    as logical dock/order metadata (no rects). Serialize via headless-owned DTOs using
    `raw()` IDs — no derives on public ID types.
  - Tests: stable JSON snapshot for a small constructed model.

- **1.3 `HeadlessBackend` command interpreter (no wildcard arm)**
  - Interpret the Phase 1 subset into `UiModel` mutations; reuse `PlatformError`;
    unsupported commands return a documented error (no silent no-op); `match` has no
    wildcard so a new variant fails to compile until handled.
  - Tests: per-command unit test of the model effect; unknown-control / duplicate-id /
    invalid-layout-rules return the correct `PlatformError` (parity with `validate_layout_rules`).

- **1.4 Headless event pump + follow-up native-event queue**
  - Pump drains commands and delivers follow-up native events to synchronous quiescence.
  - `SignalMainWindowUISetupComplete` enqueues `MainWindowUISetupComplete` as a follow-up
    event delivered *after* the current batch, not inline.
  - Tests: synchronous-quiescence fixed-point; setup-complete parity (handler gets
    `MainWindowUISetupComplete` after the batch, then its enqueued commands drain).

- **1.5 `HeadlessHarness` driver**
  - `new` / `create_window` / `start(handler, provider, initial_commands)` (drains via
    pump, non-blocking); `pump`; `wait_for(label, timeout)`; `snapshot() -> JSON`;
    `inject_raw(AppEvent)` (in-process only).
  - Tests: `Checkpoint` observed in order; `wait_for` resolves on marker; missing marker →
    timeout `Err`; snapshot JSON stable/deterministic.

- **1.6 Semantic actions with native-state transitions**
  - Implement the Spec §7 table: `set_text`, `select_row`, `select_combo`, `select_tab`,
    `toggle` (checkbox/switch), `select_radio`, `click`. Each validates against the model,
    applies the model transition, then emits the canonical `AppEvent`. Programmatic `Set*`
    commands stay event-silent. Disabled listbox rows remain selectable.
  - Tests: per-action transition+event; programmatic `Set*` silence; disabled-row-selectable;
    impossible input (missing/destroyed control, window not shown, nonexistent row) → `Err`.

- **1.7 Demo app + end-to-end test + cross-platform app-core**
  - Demo app factored into build-core (handler + initial commands from `WindowId`) vs run;
    the **app-core path compiles on non-Windows** (only the Win32 `run` is `cfg(windows)`).
  - One e2e example test: drive the harness through a realistic sequence ending in
    `Checkpoint("done")`, then assert on the JSON snapshot.

- **1.8 Phase-1 acceptance gate**
  - Verify all Spec §13 criteria (coverage; unsupported-is-explicit; validation seams
    shared; snapshot stability; setup-complete parity).
  - `clippy -D warnings` + `fmt`; bump version + `CHANGELOG.md` for the `Checkpoint`
    surface; add an `EngineeringDiary.md` entry.

Out of Phase 1 (deferred to later phases per Spec §15): `wait_until` predicate waits;
dialog responder; treeview / richedit / chart / progress / splitter; the `--headless`
stdio protocol; keyboard nav / scroll; geometry.

## Iteration log

Newest entries at the bottom. One entry per review cycle: findings → resulting Spec /
Roadmap deltas.

- 2026-05-30 — Roadmap created. Spec already revised once from
  `docs/Review.HeadlessRenderBackend.md` (semantic-action state transitions, follow-up
  event pump for `SignalMainWindowUISetupComplete`, dialog responder matching, serde
  boundary, runtime-selection platform wording, `Checkpoint` rename, parity criteria).

## Parking lot

Ideas surfaced but intentionally deferred, so they are not lost:

- RON snapshot output for Rust-side `insta` tests (model already serde-derived).
- Harness-owned executor for fully deterministic async (Spec §8, Phase 3).
- First-class `context_tag` fields on tagless dialog commands (semver impact — Spec §11).
- Optional default-on feature to strip headless from lean release builds (Spec §10).
