# Repo Instructions

## Workflow
- Build with `cargo build`.
- When a task is complete, run `cargo clippy --all-targets -- -D warnings` and then `cargo fmt`.
- For plan-driven work, write commit messages about the code change, not the plan. Follow recommended practices for teh design of the comment.
- When adding a CLI flag to `harvester_batch`, update `scripts/Start-HarvesterBatch.ps1` in the same change.
- When creating or saving plan documents, always save them to the `docs/` folder unless explicitly told otherwise. Use the prfix 'Plan.'.

## Architecture
- Preserve the host/library flow: native input -> `AppEvent` -> host state/update logic -> `PlatformCommand` -> native effect/render.
- Keep application state and business logic out of the platform layer.
- Keep native side effects isolated behind event translation and command execution seams.
- Keep entry points (`main.rs`, `mod.rs` and `lib.rs`) files as thin wrappers only.
- Keep shared constants and behavior DRY; prefer one source of truth over duplicated definitions.

## Testing
- Bug fixes should include a regression test when practical.
- Prefer tests of event translation, emitted effects, and public contracts over internal details.
- `use super::*;` is acceptable inside an inline `#[cfg(test)]` block, but extracted test files (e.g. `tests.rs`) must use explicit imports.

## Diary
- Keep `docs/EngineeringDiary.md` up to date for noteworthy implementations, important decisions, and bug fixes with reusable lessons.
- Keep diary entries short and reference concrete artifacts.
- Add new entries to the end.
