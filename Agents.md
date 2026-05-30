# Repo Instructions

## Workflow
- Build with `cargo build`.
- When a task is complete, run `cargo clippy --all-targets -- -D warnings` and then `cargo fmt`.
- For plan-driven work, write commit messages about the code change, not the plan. Follow recommended practices for teh design of the comment.
- When adding a CLI flag to `harvester_batch`, update `scripts/Start-HarvesterBatch.ps1` in the same change.
- When creating or saving plan documents, always save them to the `docs/` folder unless explicitly told otherwise. Use the prfix 'Plan.' for plans, 'Spec.' for specifications and 'Review.' for reviews.
- New handler modules must include a `#[cfg(test)]` block covering at least the pure-logic seam.
- Keep `docs/EngineeringDiary.md` up to date for noteworthy implementations, important decisions, and bug fixes with reusable lessons. See instructions in the beginning how to add entries.

## Releases
- Update `Cargo.toml` version and `CHANGELOG.md` together, in the same change.
- Bump the version only for releasable crate changes that matter to downstream users.
- Use semver intent.
- Keep `CHANGELOG.md` focused on released, user-facing changes.

## Architecture
- Preserve the host/library flow: native input -> `AppEvent` -> host state/update logic -> `PlatformCommand` -> native effect/render.
- Keep application state and business logic out of the platform layer.
- Keep native side effects isolated behind event translation and command execution seams.
- Keep entry points (`main.rs`, `mod.rs` and `lib.rs`) files as thin wrappers only.
- Keep shared constants and behavior DRY; prefer one source of truth over duplicated definitions.

## Testing
- Bug fixes should include a regression test when practical.
- Prefer tests of event translation, emitted effects, and public contracts over internal details.
- Reducer seam before Win32 syscall. When a function both (a) decides something from its inputs and (b) calls Win32 to act on that decision, split (a) into a pure function and test it. Event-translation handlers should be callable with `HWND::default()` — if they can't, the reduction is still entangled.
- `use super::*;` is acceptable inside an inline `#[cfg(test)]` block, but extracted test files (e.g. `tests.rs`) must use explicit imports.
