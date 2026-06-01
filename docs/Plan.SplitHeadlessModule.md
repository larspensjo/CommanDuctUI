# Plan: Split `src/headless.rs` into a module directory

`src/headless.rs` is ~6,745 lines. It has clean seams, so this is mechanical
extraction (no logic change). Convert `src/headless.rs` →
`src/headless/` with a thin `mod.rs` wrapper. `lib.rs` keeps
`pub mod headless;` unchanged.

After **each** step: `cargo build`, then
`cargo clippy --all-targets -- -D warnings`, then `cargo fmt`. Commit each
step separately (mechanical move, no behavior change).

## Steps

1. **Extract tests** (~2,656 lines, biggest/lowest-risk win).
   Move the `#[cfg(test)]` block (lines ~4090–end) into
   `src/headless/tests.rs`, declared `#[cfg(test)] mod tests;`.
   Per Agents.md, extracted test files use **explicit imports**, not
   `use super::*;` — build the real import list.

2. **Extract snapshots** (~860 lines).
   Move all `*Snapshot` structs and their `From` impls into
   `src/headless/snapshot.rs`. Dependency direction is clean (snapshots
   depend on state, not vice versa).

3. **Extract backend** (~1,900 lines).
   Move `HeadlessBackend` and its command handlers/validators into
   `src/headless/backend.rs`. May later sub-split (dialogs, tree, etc.);
   one module is a fine first step.

4. **Extract protocol types** (~355 lines).
   Move `Protocol*` wire types + `into_runtime` conversions (and the
   dialog types they pair with) into `src/headless/protocol.rs`.

5. **Extract state model**.
   Move `WindowState`, `ControlState`, `ControlKind`, `TreeItemNode`,
   `MenuNode`, and tree/menu helpers into `src/headless/state.rs`.

6. **Finalize `mod.rs`**.
   `mod.rs` keeps `HeadlessHarness` (public API) plus `mod` declarations
   and `pub use` re-exports so the external API is unchanged.

## Minimal alternative

If a full six-way split is too much: do **steps 1 + 2 only**. That takes the
main file from ~6,745 → ~3,200 lines and isolates the two most mechanical
regions.

## Notes

- Branch: `feature/headless-backend`. Keep extraction as its own commit(s),
  separate from feature work, so the large move diff is easy to review.
- Verify the public API (`pub mod headless`, `HeadlessHarness`) still compiles
  on every platform per `docs/Spec.HeadlessRenderBackend.md`.
