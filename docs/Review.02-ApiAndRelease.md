# Phase 2 — Public API, Documentation, crates.io Readiness

Phase 2 of [Plan.ThoroughReview](Plan.ThoroughReview.md). Assesses the crate's
surface against crates.io publication requirements and common API hygiene.

Scope: [src/lib.rs](../src/lib.rs), [Cargo.toml](../Cargo.toml),
[Readme.md](../Readme.md), [CHANGELOG.md](../CHANGELOG.md), [LICENSE](../LICENSE),
public surfaces in [src/types.rs](../src/types.rs),
[src/styling_primitives.rs](../src/styling_primitives.rs),
[src/error.rs](../src/error.rs), [src/app.rs](../src/app.rs).

> **Revision note.** This document was revised in-place after a cross-check
> (see [Review.02-ReviewCheck.md](Review.02-ReviewCheck.md)). Severities were
> recalibrated so that "blocker" is reserved for issues that actually prevent
> a correct publish, and a few items were merged, downgraded, or retired as
> strategy advice rather than defects. Affected findings: F-02-002, F-02-004,
> F-02-005, F-02-006, F-02-008, F-02-009, F-02-012, F-02-013. One new finding
> was added: F-02-014 (package contents hygiene).
>
> **Implementation note (2026-04-18).** Follow-up work fixed F-02-001,
> F-02-002, F-02-003, F-02-004, F-02-006, F-02-007, F-02-009, F-02-010,
> F-02-011, and F-02-014. F-02-012 remains open as future API-shaping work.

## Summary verdict

The crate is **close to — but not yet — ready for crates.io publication**.
It compiles cleanly, has a coherent public surface, and `cargo package`
succeeds. The real blockers are small and manifest-level; most of the rest
is polish that improves the first-impression but does not prevent a publish.

**Genuine publishability blockers:**

1. License files do not match the `MIT OR Apache-2.0` SPDX claim, and the
   README links to missing license files (F-02-001).
2. `cargo package` ships local editor, plan, and review material that should
   not reach crates.io (F-02-014).

**Strong publish-quality issues** (not blockers, but visible on docs.rs from
the first release):

3. No crate-level rustdoc (`//!`) in `lib.rs`; many public items use plain
   `//` or `/* */` comments that rustdoc ignores (F-02-003).
4. Growth-oriented public enums (`PlatformCommand`, `AppEvent`, `StyleId`,
   `MessageSeverity`) lack `#[non_exhaustive]` (F-02-004).
5. Recommended `Cargo.toml` metadata (`repository`, `keywords`, `categories`,
   `rust-version`, etc.) is absent (F-02-002).

**Policy decisions, not defects** (previously framed as blockers):

- Whether to publish first at `1.0.x` or reset to `0.x` is a product call,
  not a repo defect. Captured as optional advice; see F-02-005.
- Whether the public API must be flat at the crate root is a policy call;
  the current shape is reachable because `mod types` and `mod error` are
  already public. Downgraded; see F-02-006, F-02-013.

## Artifacts reviewed

### Public re-exports from `src/lib.rs`

```rust
pub use app::PlatformInterface;
pub use error::Result as PlatformResult;
pub use styling_primitives::{Color, ControlStyle, FontDescription, FontWeight, StyleId};
pub use types::{
    AppEvent, BadgeDescriptor, ChartDataPacket, ChartLineData, ChartLineEmphasis, CheckState,
    FormButtons, FormDialogDescriptor, FormField, FormFieldValue, FormFileExistsWarning, FormRow,
    FormTextValidation, ListBoxItemDescriptor, ListBoxItemId, MessageSeverity, PlatformCommand,
    PlatformEventHandler, TreeItemDescriptor, TreeItemId, UiStateProvider, WindowConfig, WindowId,
};
```

`PlatformError` itself is not re-exported — only the `PlatformResult` alias is.
`error` is `pub mod`, so `commanductui::error::PlatformError` remains a valid
path for downstream code; the only question is ergonomics.

`ControlId`, `MenuActionId`, `MenuItemConfig`, `DockStyle`, `LayoutRule`,
`TreeItemMarkerKind`, `SplitterOrientation`, and `LabelClass` are each `pub`
in `types.rs` but not re-exported from `lib.rs`. Same story: reachable via
`commanductui::types::...`, not reachable via `commanductui::...` directly.
See F-02-006 for the ergonomics tradeoff.

### Cargo metadata (current state)

Empirical check via `cargo package --allow-dirty --list` produces this
warning:

```
warning: manifest has no documentation, homepage or repository
```

Only these three fields cause the cargo publish warning. Everything else in
the table below is "recommended" rather than "required".

| Field | Present | Cargo-warning trigger | Notes |
|-------|---------|:---:|-------|
| `name` | ✓ | | `commanductui` |
| `version` | ✓ | | `1.0.8` |
| `edition` | ✓ | | `2024` |
| `description` | ✓ | | |
| `license` | ✓ | | `MIT OR Apache-2.0` — see F-02-001 |
| `repository` | ✗ | ✓ | warned by `cargo package` |
| `homepage` | ✗ | ✓ | warned by `cargo package` |
| `documentation` | ✗ | ✓ | warned by `cargo package` |
| `authors` | ✗ | | optional, no warning |
| `readme` | ✗ | | auto-detected on this machine; see note below |
| `keywords` | ✗ | | crates.io limits to 5 |
| `categories` | ✗ | | candidates: `gui`, `os::windows-apis` |
| `rust-version` | ✗ | | edition 2024 needs ≥ 1.85 |
| `exclude`/`include` | ✗ | | see F-02-014 |

**`readme` note.** The tracked file is `Readme.md`, and `cargo package --list`
reveals that cargo also ships a generated `README.md` alongside it. Setting
`readme = "Readme.md"` (or renaming the file and pointing at `README.md`)
makes the behavior explicit and portable across case-sensitive filesystems.

### Feature flags

No features are defined. Given the lean dependency set (only `log` +
`windows` behind `cfg(target_os = "windows")`), this is acceptable. No
finding — just documenting the decision.

### LICENSE files

- `LICENSE` — MIT only (Copyright 2025 Lars Pensjö).
- `LICENSE-MIT` — **absent**.
- `LICENSE-APACHE` — **absent**.
- `Readme.md` lines 168-169 link to both missing files.

### CHANGELOG discipline

[CHANGELOG.md](../CHANGELOG.md) is well maintained per release: each entry is
user-facing and scoped. Minor style gaps noted as F-02-011 (low priority).

### Examples directory

Missing. The README embeds a ~60-line example that is not compiled by the
test suite, so it can silently drift. See F-02-009.

### CI scaffolding

No `.github/` directory. See F-02-010.

### `cargo doc` output

`cargo doc --no-deps --document-private-items` completes in ~1.2 s with
zero warnings. That means items technically satisfy rustdoc's lint — not
that they are documented. See F-02-003.

### Rustdoc comment-style audit

Quick grep across the main public files:

| File | `///` lines | `//` comment lines |
|------|-------------|---------------------|
| `src/types.rs` | 39 | 32 |
| `src/app.rs` | 0 | 5 + several `/* */` blocks |
| `src/lib.rs` | 0 | 0 (only `/* */` block) |
| `src/error.rs` | 1 | 6 |
| `src/styling_primitives.rs` | 0 | 0 (only `/* */` blocks) |

Most doc-intent prose on public types sits inside `//` or `/* */` comments
that produce **no rustdoc output**.

### `cargo package` contents

`cargo package --allow-dirty --list` includes files that should not ship to
crates.io: `.claude/settings.local.json`, `.vscode/tasks.json`, `CLAUDE.md`,
`Agents.md`, all `docs/Plan.*`, all `docs/Review.*`. See F-02-014.

### Semver surface tripwires

- `PlatformCommand` is a **closed** enum today (51 variants); every added
  command in a 1.x release after publication is a SemVer break without
  `#[non_exhaustive]`.
- `AppEvent` — same, 27 variants.
- `StyleId` — same. Growth-oriented: the CHANGELOG shows 5+ additions since
  0.10.3.
- `MessageSeverity` — same.
- Other public enums (`CheckState`, `FontWeight`, `DockStyle`,
  `TreeItemMarkerKind`, `SplitterOrientation`, `LabelClass`,
  `ChartLineEmphasis`, `FormRow`, `FormField`, `FormTextValidation`,
  `FormFieldValue`) may be intentionally closed. Each should get a
  conscious decision rather than a blanket markup. See F-02-004.
- `Color { pub r, pub g, pub b }` — adding alpha later would break it.
  Literal construction is valuable; document the decision.
- Newtype identifier families are inconsistent (see F-02-007).

## Findings

### F-02-001: License files do not match the `MIT OR Apache-2.0` SPDX claim
- **Severity:** Major
- **Dimension:** release
- **Status:** fixed
- **Location:** [Cargo.toml:5](../Cargo.toml), [LICENSE](../LICENSE), [Readme.md:166-169](../Readme.md)
- **Observation:** `Cargo.toml` declares a dual MIT-or-Apache license, but
  only a single MIT `LICENSE` file exists. `Readme.md` links to
  `LICENSE-MIT` and `LICENSE-APACHE`, which are missing. (This also covers
  what was originally called out separately as F-02-008.)
- **Why it matters:** The mismatch is user-hostile: someone auditing the
  crate will find the declared dual license unverifiable, and the README
  links are broken for every visitor. crates.io renders the README verbatim.
- **Recommendation:** Decide the licensing intent first. Easiest fix: drop
  the Apache option — set `license = "MIT"` in `Cargo.toml`, fix the README
  accordingly. Alternative: add the Apache-2.0 text as `LICENSE-APACHE`,
  rename `LICENSE` to `LICENSE-MIT`, and add the standard Rust dual-license
  footer `// SPDX-License-Identifier: MIT OR Apache-2.0` to `lib.rs`.

### F-02-002: Recommended `Cargo.toml` metadata is incomplete
- **Severity:** Minor
- **Dimension:** release
- **Status:** fixed
- **Location:** [Cargo.toml](../Cargo.toml)
- **Observation:** `cargo package` prints one warning:
  `manifest has no documentation, homepage or repository`. These three
  fields — plus `keywords`, `categories`, `authors`, `rust-version`, and an
  explicit `readme` — are absent. `readme` auto-detection does work on this
  machine but depends on filesystem case sensitivity and cargo behaviour.
- **Why it matters:** A crates.io listing without search metadata
  (`keywords`, `categories`) is harder to discover. Omitting `rust-version`
  silently assumes edition 2024's baseline (1.85) for downstream users.
  None of this blocks `cargo publish`.
- **Recommendation:** Split the fix into two tiers.
  Tier 1 (silences the cargo warning): `repository`, `homepage`,
  `documentation`.
  Tier 2 (quality-of-life for crates.io): `authors`,
  `keywords = ["win32", "ui", "gui", "windows", "declarative"]`,
  `categories = ["gui", "os::windows-apis"]`, `rust-version = "1.85"`, and
  an explicit `readme = "Readme.md"` (or rename to `README.md` and reference
  it).

### F-02-003: No crate-level rustdoc; most public items use non-rustdoc comments
- **Severity:** Major
- **Dimension:** api
- **Status:** fixed
- **Location:** [src/lib.rs:1-11](../src/lib.rs), [src/app.rs:1252-1261](../src/app.rs),
  [src/types.rs](../src/types.rs), [src/styling_primitives.rs](../src/styling_primitives.rs)
- **Observation:** `lib.rs` opens with a `/* */` block that is not `//!`, so
  docs.rs will render the crate root with no description. Public types like
  `PlatformInterface`, `Color`, `ControlStyle`, `StyleId`, `WindowId`,
  `CheckState`, and most `AppEvent`/`PlatformCommand` variants use `//` line
  comments instead of `///`. rustdoc silently ignores them.
- **Why it matters:** docs.rs is the primary discoverability surface for a
  crate. Rendering bare signatures with no prose makes the API look
  undocumented even though the author wrote extensive prose. Users will
  write wrong code because they cannot see the invariants that already exist
  in the source.
- **Recommendation:** Convert `/* */` and leading `//` doc-intent comments to
  `//!` (at the top of modules) and `///` (on items). Start with `lib.rs`,
  then the types re-exported from the prelude. A mechanical pass with
  `#![warn(missing_docs)]` on `lib.rs` will surface remaining gaps.

### F-02-004: Growth-oriented public enums lack `#[non_exhaustive]`
- **Severity:** Major
- **Dimension:** api
- **Status:** fixed
- **Location:** [src/types.rs:229](../src/types.rs) (`AppEvent`),
  [src/types.rs:537](../src/types.rs) (`PlatformCommand`),
  [src/styling_primitives.rs:58](../src/styling_primitives.rs) (`StyleId`),
  [src/types.rs:390](../src/types.rs) (`MessageSeverity`).
- **Observation:** None of the public enums are marked `#[non_exhaustive]`
  (zero `non_exhaustive` occurrences across `src/`). The CHANGELOG shows
  that `PlatformCommand`, `AppEvent`, and `StyleId` grow with nearly every
  release.
- **Why it matters:** Every future variant on any of these enums after the
  first `1.0.0` publish becomes a breaking change. For enums that the
  project already demonstrates a pattern of extending, this is a semver
  trap that is free to avoid today.
- **Recommendation:** Apply `#[non_exhaustive]` only to the enums the
  project intends to keep extending. The clearest candidates from the
  CHANGELOG are `PlatformCommand`, `AppEvent`, `StyleId`, and
  `MessageSeverity`. For each other public enum (`CheckState`, `FontWeight`,
  `DockStyle`, `TreeItemMarkerKind`, `SplitterOrientation`, `LabelClass`,
  `ChartLineEmphasis`, `FormRow`, `FormField`, `FormTextValidation`,
  `FormFieldValue`), make a conscious decision: if the set is intended to
  stay closed (e.g. `FontWeight`), leave it as-is and optionally document
  the intent. Apply the marker before the first crates.io publish; after
  publish it requires a major bump.

### F-02-005: Starting at `1.0.x` vs resetting to `0.x` — release strategy advice
- **Severity:** Nit
- **Dimension:** release
- **Status:** wontfix (advisory only; product decision, not a defect)
- **Location:** [Cargo.toml:3](../Cargo.toml), [CHANGELOG.md](../CHANGELOG.md)
- **Observation:** The manifest is at `version = "1.0.8"`. The project
  history suggests the crate has not yet been published to crates.io, but
  that is an external-system fact the repository itself cannot prove.
- **Why it matters:** This is a product decision, not a defect. A 1.x first
  publish commits to semver stability from day one; a 0.x first publish
  absorbs first-user feedback without ceremony. Both are valid.
- **Recommendation:** No action required. If the author wants to absorb
  post-publication feedback without 2.0 bumps, consider a one-time reset
  to `0.1.0` before the first `cargo publish`. Otherwise, keep the
  current line and treat the non-exhaustive markup in F-02-004 as the
  main hedge.

### F-02-006: Several public types are only reachable via internal module paths
- **Severity:** Minor
- **Dimension:** api
- **Status:** fixed
- **Location:** [src/lib.rs:36-41](../src/lib.rs), [src/types.rs](../src/types.rs)
- **Observation:** `ControlId`, `MenuActionId`, `MenuItemConfig`, `DockStyle`,
  `LayoutRule`, `TreeItemMarkerKind`, `SplitterOrientation`, and `LabelClass`
  are each `pub` in `types.rs` but not in the `lib.rs` prelude re-export.
  Because `mod types` is `pub`, they remain reachable as
  `commanductui::types::ControlId` — the issue is ergonomics, not access.
  Same observation applies to `PlatformError` (originally called out
  separately as F-02-013; now folded here).
- **Why it matters:** Downstream code that writes
  `PlatformCommand::CreateButton { control_id, .. }` still needs to import
  `ControlId` from somewhere. Forcing every such import through the
  `types::` submodule inconsistently surfaces some API types at the root
  and others only via modules, which reads as an incomplete re-export list.
- **Recommendation:** Decide whether the public API is intended to be flat
  at the crate root. If yes: re-export every type reachable from a public
  signature (all field types in `PlatformCommand` / `AppEvent`, plus
  `PlatformError`). If no: document the convention ("these live in
  `types`"). Either way, the choice should be explicit; today it reads as
  unintentional.

### F-02-007: Identifier newtypes have inconsistent inner visibility and width
- **Severity:** Minor
- **Dimension:** api
- **Status:** fixed
- **Location:** [src/types.rs:21](../src/types.rs), [src/types.rs:39](../src/types.rs),
  [src/types.rs:43](../src/types.rs), [src/types.rs:70](../src/types.rs),
  [src/types.rs:103](../src/types.rs)
- **Observation:** `WindowId(pub(crate) usize)` vs `TreeItemId(pub u64)` vs
  `ListBoxItemId(pub u64)` vs `ControlId(pub i32)` vs `MenuActionId(pub u32)`.
  `WindowId` is the only one a downstream crate cannot construct directly;
  every other can be forged or deconstructed with `.0`. The README example
  at [Readme.md:88](../Readme.md#L88) uses `WindowId(1)`, which will not
  compile for a real consumer.
- **Why it matters:** Consumers who mirror the pattern of one id type will
  be surprised when the next one behaves differently. Inconsistent widths
  (`i32`, `u32`, `u64`, `usize`) leak implementation choices (Win32 menu
  ids are `u16`, control ids are `i32`) into the application layer.
- **Recommendation:** Decide once: either all ids are opaque
  (`pub(crate)` inner) with `new` + `raw` accessors (like `WindowId`), or
  all are transparent newtypes (`pub` inner). Pick a uniform width per
  semantic. Fix the README example either way.

### F-02-008: (merged into F-02-001)
- **Severity:** —
- **Dimension:** —
- **Status:** wontfix (see F-02-001)
- **Location:** —
- **Observation:** The broken README license links originally listed here
  are the same concrete problem as F-02-001. Keeping the ID reserved for
  traceability; no separate action.

### F-02-009: No `examples/` directory; README example is broken
- **Severity:** Minor
- **Dimension:** release
- **Status:** fixed
- **Location:** repository root, [Readme.md:65-127](../Readme.md#L65)
- **Observation:** The README embeds a 60-line working example but nothing
  compiles it. The example uses `WindowId(1)` directly, which would not
  compile against `WindowId(pub(crate) usize)` from a downstream crate.
- **Why it matters:** crates.io surfaces `examples/` directories prominently
  in the "Examples" tab. A CI-compiled example is a regression net for the
  public surface — it would have caught F-02-007 already.
- **Recommendation:** Move the README example to `examples/hello_window.rs`,
  fix the compile error, and add `cargo build --examples` to CI. Keep a
  smaller excerpt in the README linking to the full file.

### F-02-010: No CI workflow
- **Severity:** Minor
- **Dimension:** release
- **Status:** fixed
- **Location:** repository root (`.github/` absent)
- **Observation:** No GitHub Actions or equivalent configuration.
- **Why it matters:** Without CI a publish-blocking regression (clippy
  warning, test failure, compile break on non-Windows) lands unnoticed.
  For a Win32-only crate, CI is also the only way to ensure the
  `cfg(not(target_os = "windows"))` stub stays compilable.
- **Recommendation:** Add a minimal workflow that runs
  `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check`, and `cargo check --target x86_64-unknown-linux-gnu`.

### F-02-011: CHANGELOG lacks an `Unreleased` section and releases lose dates post-1.0
- **Severity:** Nit
- **Dimension:** release
- **Status:** fixed
- **Location:** [CHANGELOG.md](../CHANGELOG.md)
- **Observation:** Entries for 0.x releases carry dates (e.g.
  `## 0.9.0 - 2026-03-24`); entries 1.0.0-1.0.8 omit them. There is no
  `## Unreleased` header where in-flight changes can accumulate.
- **Why it matters:** Style / process preference. Not a release-readiness
  issue.
- **Recommendation:** Keep release dates on every tagged entry. Add an
  `## Unreleased` section at the top of the file that captures the next
  release's changes as they land.

### F-02-012: `PlatformError` variants discard structured cause information
- **Severity:** Nit
- **Dimension:** api
- **Status:** open (future API shaping)
- **Location:** [src/error.rs:10-25](../src/error.rs)
- **Observation:** Every non-Win32 variant carries only a `String`. The
  TODO on line 8 already acknowledges this.
- **Why it matters:** A public error type is harder to evolve later than
  most types — downstream `match` arms become exhaustive promises. This is
  a design preference, not a concrete defect: the current shape is
  internally consistent and works.
- **Recommendation:** Optional pre-1.0 improvement. If pursued, replace
  each `String` payload with a structured struct (e.g.
  `InvalidHandle { kind: HandleKind, id: u64 }`) and add `#[non_exhaustive]`
  to `PlatformError` itself.

### F-02-013: (merged into F-02-006)
- **Severity:** —
- **Dimension:** —
- **Status:** wontfix (see F-02-006)
- **Location:** —
- **Observation:** The `PlatformError` re-export concern originally listed
  here is the same ergonomics question as the other `types::`/`error::`
  types in F-02-006. Keeping the ID reserved for traceability.

### F-02-014: `cargo package` ships local editor, plan, and review material
- **Severity:** Major
- **Dimension:** release
- **Status:** fixed
- **Location:** [Cargo.toml](../Cargo.toml), repository root
- **Observation:** `cargo package --allow-dirty --list` includes files that
  have nothing to do with the library contract:
  - `.claude/settings.local.json`
  - `.vscode/tasks.json`
  - `CLAUDE.md`, `Agents.md`
  - `docs/Plan.FirstCleanupPass.md`, `docs/Plan.MigrateToGenericMenuAction.md`,
    `docs/Plan.ThoroughReview.md`
  - all `docs/Review.*.md` (this review document, the Phase 0/1 reports,
    `Review.Backlog.md`, the review-check)
- **Why it matters:** Every byte shipped to crates.io is immutable for that
  version. Tool configs leak author environment. Review documents reference
  commit hashes and internal discussion. Plan documents describe
  unreleased work. None of this serves a consumer.
- **Recommendation:** Curate package contents explicitly. Either whitelist
  via `include = ["src/**", "Cargo.toml", "LICENSE*", "README*", "CHANGELOG.md"]`,
  or blacklist via `exclude = [".claude/**", ".vscode/**", "CLAUDE.md", "Agents.md", "docs/Plan.*", "docs/Review.*"]`.
  Verify with `cargo package --list` before the first publish.

## Pre-publication checklist

For a first `cargo publish` pass the following should land — in this order:

1. **License hygiene** (F-02-001): pick MIT-only or dual, add the matching
   LICENSE file(s), fix the README.
2. **Package contents** (F-02-014): add `include`/`exclude` and verify with
   `cargo package --list`.
3. **Cargo metadata** (F-02-002): add at minimum the three fields that
   `cargo package` warns about (`repository`, `homepage`, `documentation`)
   plus `keywords`, `categories`, `rust-version`.
4. **Non-exhaustive markers** (F-02-004): apply to the growth-oriented
   enums (`PlatformCommand`, `AppEvent`, `StyleId`, `MessageSeverity`).
   Decide and document the intent for the other public enums.
5. **Rustdoc conversion** (F-02-003): `//!` at module tops, `///` on items.
   Enable `#![warn(missing_docs)]` on the crate root once obvious gaps are
   filled.
6. **Re-export policy** (F-02-006): decide flat-root vs submodule-pathed;
   make the chosen shape consistent.
7. **Identifier consistency** (F-02-007): pick one convention for newtypes.
8. **Examples + CI** (F-02-009, F-02-010): one compiled example + a minimal
   workflow.
9. **Optional polish**: changelog `Unreleased` section (F-02-011), error
   shape (F-02-012), version line strategy (F-02-005).

Items 4, 6, 7, 12 are breaking changes that are free now and expensive
after the first publish. The rest can land incrementally.

## Out of scope for Phase 2

- `cargo doc` **warnings-as-errors** enforcement — meaningful only after
  the rustdoc conversion pass in F-02-003 lands.
- The actual content of `StyleId` (deferred to Phase 4 code quality).
- Thread-safety of `PlatformInterface` (Phase 3 correctness).
- Whether `Send + Sync + 'static` on `PlatformEventHandler` is the right
  bound (Phase 4 ergonomics).
