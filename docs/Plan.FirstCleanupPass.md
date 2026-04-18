# Plan.FirstCleanupPass

## Goal

Land the first low-risk cleanup pass from `Review.01-Architecture.md` without
changing runtime behavior beyond fixing the latent `ParsedControlStyle` clone
bug. This pass should make the public contract clearer and remove one
architecture inconsistency before any larger refactors.

## Scope

Included findings:

- `F-01-003`: remove `Clone` from `ParsedControlStyle`
- `F-01-002`: document the UI-thread and re-entrancy contract on
  `PlatformEventHandler`
- `F-01-010`: route `listbox_handler` styling imports through `crate::styling`
- `F-01-011`: document why the handler/provider fields use
  `Mutex<Option<Weak<Mutex<dyn ...>>>>`

Explicitly out of scope for this pass:

- event loop wake-up redesign (`F-01-001`)
- moving event construction per control (`F-01-004`)
- moving command execution per control (`F-01-005`)
- large file splits (`F-01-006` through `F-01-009`)

Already applied before this pass:

- the architecture-guidance decision entry in `docs/EngineeringDiary.md`

## Design Rules

- No API redesign unless required for the documentation work.
- Preserve current message ordering and threading behavior.
- Keep changes small enough to review as one PR.
- Prefer comments and rustdoc where the current shape is intentional.

## Implementation Steps

### 1. Remove the latent double-free hazard

Target:

- `src/styling_windows.rs`

Changes:

- Remove `Clone` from `ParsedControlStyle`.
- Keep ownership through `Arc<ParsedControlStyle>` as the only shared-style
  mechanism.
- Confirm there are no `.clone()` call sites that depend on the derived impl.

Checks:

- Search for `ParsedControlStyle` construction and usage.
- Verify no code or tests require `Clone`.
- Confirm the `listbox_handler` test that builds a `ParsedControlStyle`
  struct literal still compiles unchanged; this pass removes only the derive.

Expected outcome:

- `ParsedControlStyle` can no longer be accidentally copied while still owning
  GDI handles that are freed in `Drop`.

### 2. Document the event-handler threading contract

Targets:

- `src/types.rs`
- optionally `src/app.rs` if cross-references help clarity

Changes:

- Add rustdoc to `PlatformEventHandler` explaining:
  - `handle_event` runs on the UI thread
  - it is called synchronously from native message dispatch
  - handlers are expected not to block because blocking stalls native message
    dispatch and can hang the UI
  - enqueueing `PlatformCommand` values is the supported way to trigger UI work
  - synchronous re-entrancy back into the platform layer is unsupported

Checks:

- Keep wording specific to current behavior in `Win32ApiInternalState::send_event`.
- Avoid promising future async semantics.
- Cross-reference `send_event` where it helps readers verify the contract.

Expected outcome:

- The most important public contract is explicit at the trait boundary instead
  of being discoverable only by reading `app.rs` and `window_common.rs`.

### 3. Comment the unusual handler/provider storage shape

Target:

- `src/app.rs`

Changes:

- Add a primary comment on the `UiStateProviderHolder` alias describing:
  - the host owns the strong reference
  - the library stores a `Weak` to avoid ownership cycles
  - the outer `Mutex` protects installation and access
  - the inner `Mutex` remains the host-controlled synchronization boundary
- Add a short field-level comment for `application_event_handler` only if the
  alias-side comment does not make its matching shape obvious.

Checks:

- Keep comments factual and brief.
- Do not change the type in this pass.

Expected outcome:

- A future maintainer can see that the triple-wrapped shape is deliberate rather
  than accidental.

### 4. Normalize the styling import path

Target:

- `src/controls/listbox_handler.rs`

Changes:

- Replace direct `crate::styling_windows::ParsedControlStyle` import with
  `crate::styling::{ParsedControlStyle, StyleId}`.
- Remove the now-redundant `crate::styling_primitives::StyleId` import.

Checks:

- Compare against other handlers that already use the `styling` alias.
- Confirm `listbox_handler` remains Windows-only through parent-module gating or
  unconditional Win32 usage, since `styling_stub` does not expose
  `ParsedControlStyle`.
- Ensure the module still compiles on the active Windows target path.

Expected outcome:

- `listbox_handler` follows the same platform-indirection boundary as the rest
  of the crate.

### 5. Update review tracking

Targets:

- `docs/Review.Backlog.md`
- `docs/Review.01-Architecture.md`

Changes:

- Mark the four included findings as `fixed` once code lands.
- If any item partially lands, mark it `in-progress` and note what remains.

Checks:

- Keep wording consistent between backlog and source review docs.

Expected outcome:

- The review documents reflect the current state of the codebase.

## Verification Plan

Run in this order:

1. `cargo build`
2. `cargo test`
3. `cargo clippy --all-targets -- -D warnings`
4. `cargo fmt`

Then perform a quick source audit:

- confirm `ParsedControlStyle` no longer derives `Clone`
- confirm there are no remaining `ParsedControlStyle` clone call sites, using
  `rg`
- confirm `PlatformEventHandler` rustdoc matches actual `send_event` behavior
- confirm `listbox_handler` imports through `crate::styling`
- confirm review statuses were updated in both review documents

## Risks

- Rustdoc wording could overstate guarantees. Keep it aligned with current
  implementation only.
- `cargo fmt` may reflow unrelated comments in touched files. Review diffs
  before finalizing.
- Updating review status before code lands would be inaccurate. Do that only in
  the implementation PR, not in this planning-only step.

## Suggested Commit Shape

Prefer one implementation PR with these commits:

1. `Remove ParsedControlStyle Clone and clarify event-thread contract`
2. `Normalize styling imports and document handler ownership shape`
3. `Update review backlog for first cleanup pass`
