# Engineering Diary

Purpose: durable project memory for AI-assisted development.

How to use:
- Add an entry when a noteworthy implementation lands.
- Add an entry for every bug fix, including lessons learned and prevention.
- Add an entry for important decisions and tradeoffs.
- Write entries so they still make sense if plans or temporary review docs are deleted later.
- Keep entries concise and reference concrete artifacts.
- New entries goes to the end of the file.

## Entry Template

## YYYY-MM-DD - Short title
Type: Implementation | Bug Fix | Decision
Context: Why this change happened.
Change: What was implemented/changed.
Lessons Learned: (required for Bug Fix)
Prevention: (required for Bug Fix)
Refs: path/to/file.rs, test_name, commit abc1234

## 2026-04-18 - Platform adapter architecture guidance
Type: Decision
Context: The repo-level architecture rule was phrased like an application reducer pattern, but this crate acts as a Win32 platform adapter.
Change: Updated `Agents.md` to define the architectural boundary as `native input -> AppEvent -> host state/update logic -> PlatformCommand -> native effect/render`, and aligned testing guidance with that boundary.
Refs: Agents.md

## 2026-04-18 - Style ownership and event-thread contract clarified
Type: Bug Fix
Context: `ParsedControlStyle` could be accidentally cloned despite owning GDI handles, and key platform-threading assumptions were only implicit in the code.
Change: Removed `Clone` from `ParsedControlStyle`, documented the UI-thread event contract on `PlatformEventHandler`, and normalized `listbox_handler` styling imports.
Lessons Learned: Small boundary-focused fixes are worth landing early when they remove future footguns without forcing a wider refactor.
Prevention: Treat native-resource owner types and thread-affinity contracts as explicit review items, not implicit assumptions.
Refs: src/styling_windows.rs, src/types.rs, src/app.rs, src/controls/listbox_handler.rs

## 2026-04-18 - Crate publish surface tightened for first release
Type: Decision
Context: The phase 2 API/release review found that the crate was close to publishable but still exposed an inconsistent root API, shipped author-only files in `cargo package`, and rendered sparse docs.rs output despite existing prose in source comments.
Change: Curated `Cargo.toml` package metadata and include rules, switched the declared license to match the shipped MIT license file, added docs.rs target metadata plus a CI workflow and compiled example, re-exported the remaining public support types from `src/lib.rs`, standardized public identifier wrappers on constructor/accessor helpers, and converted key public prose to rustdoc-visible comments.
Refs: Cargo.toml, src/lib.rs, src/types.rs, src/styling_primitives.rs, src/error.rs, src/app.rs, Readme.md, examples/hello_window.rs, .github/workflows/ci.yml, CHANGELOG.md
