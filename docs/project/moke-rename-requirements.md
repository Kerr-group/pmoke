# moke Rename / kerr & Vm Addition: Requirements (v0.1)

Status: Draft v1 (2026-09-12)
Audience: pmoke maintainer and implementers
Related: `docs/project/requirements.md` (stable requirements. This document holds the
working requirements for this change; reflect the delta into the stable requirements
once finalized.)

pmoke is a Rust 2024 workspace for pulsed-field MOKE measurement workflows.
This change renames the Kerr-analysis concept from `kerr` to `moke` and adds the true
rotation angle `angle` plus the monitor voltage `Vm` as new output quantities.
No code is copied from elsewhere.

---

## 1. Background assets

| Asset | Role | Treatment in this change |
| --- | --- | --- |
| `src/kerr/`, `src/commands/kerr.rs`, `crates/pmoke-analysis-core/src/kerr.rs` | Implementation under rename | Edit targets for rename + new quantities |
| `src/config.rs`, `crates/pmoke-config-core/src/model.rs`, `xtask/src/config_schema.rs` | Owners of the config contract | Bump schema to v6, implement the `[moke]` section + legacy `[kerr]` aliasing |
| `docs/project/requirements.md` | Stable requirements | Reference only. Reflect the delta once this change is finalized |
| `website/content/docs/{en,ja}/`, generated references, `website/public/config.schema.json` | Public contract | Regenerate via `cargo xtask docs-export`. Hand-editing prohibited |
| Python comparison utilities under `scripts/` | Reference | Reference only. Follow up separately if quantity names affect them |

Current locations of the `kerr` concept (surveyed): config `[kerr]` section
(`sensor` / `method` = `standard|harmonics` / `factor`), CLI `pmoke kerr`, run stage /
artifact kind / `published_through` value `"kerr"`, `analysis/kerr/` directory,
`kerr_results.csv`, headers `Ch{N} Kerr angle (rad)` (`KERR_HEADER`), plots
`ch{N}_kerr.png`, NPY export targets, WASM `kerr_angle_rad`, synthetic
`kerr_angle_rad`, tolerance `kerr_rel`, and numerous test assertions.

## 2. Decisions (D)

- D1 The rename scope is a full concept rename. Covered: quantity display names,
  config section, CLI command, stage / artifact kinds, directory and file names,
  module / function / constant names, WASM and Python APIs, web display, JA/EN
  documentation, generated references.
- D2 New quantity definitions: `angle` = Kerr rotation angle, `Vm` = monitor voltage.
- D3 Backward compatibility via schema version bump + legacy-name aliasing. No
  breaking change.
- D4 No Rust-first staging: WASM, Python, web, documentation, and generated
  outputs are updated consistently in the same change.
- D5 Identifiers use lowercase `moke`, `angle`, and case-sensitive `Vm` throughout.
  New directory names, file names, and identifiers must read as `moke` or `angle`
  as appropriate; the string `kerr` is prohibited in all new names. Legacy
  `kerr` strings survive only inside compatibility shims: the v5→v6 migration
  reader, the deprecated CLI alias, the legacy run-directory reader, and legacy
  fixture data. Human-language prose (help text, field descriptions, UI
  explanations) may still use "Kerr angle" / "Kerr角度" for the quantity.
- D6 Generated outputs (`website/generated/`, `config.schema.json`, CLI/config MDX)
  belong to `cargo xtask docs-export`. Never hand-edit them.
- D7 The renamed `moke` analysis emits exactly two quantity columns: `angle`
  (Kerr rotation) and `Vm`. No `moke` column is created; `moke` is the pipeline/artifact
  name, not an output quantity.
- D8 Quantity formulas (user-specified):
  - `angle` (standard) = `factor * 0.5 * atan(jn(2,2*phim)*LI1_in /
    (jn(1,2*phim)*LI2_in))`, identical to the legacy kerr computation.
  - `angle` (harmonics) = existing `calculate_harmonics_kerr`
    (a2/a3/a4/a6, factor, representative depth x0), unchanged.
  - `Vm` (standard) = `0.5 * sqrt((LI1_in/jn(1,2*phim))^2 +
    (LI2_in/jn(2,2*phim))^2)`, where `jn` is the Bessel function of the first
    kind and `LI*_in` are lock-in in-phase harmonic amplitudes.
  - `Vm` (harmonics) = `0.5 * sqrt((LI3_in/jn(3,x0))^2 + (LI2_in/jn(2,x0))^2)`
    with x0 = representative modulation depth.
  - Convention note: `LI*_in` are peak harmonic amplitudes (`Xk = b_k`,
    `Yk = a_k`); before this change they were half-amplitudes (`/2`). The
    formulas above are unchanged: with peak-amplitude inputs they now yield
    the true `Vm` (previously `Vm/2`, the 1/2 was double-counted against the
    Bessel-expansion factor 2). `angle` is scale-free and unchanged.

## 3. Provisional defaults (confirm: A)

- A1 The `moke` pipeline keeps the legacy Kerr-analysis computation for the `angle`
  column unchanged (D8). Only names, labels, and artifact paths change;
  `Vm` is the sole newly computed quantity. Rationale: angle correctness is
  already covered by existing tests.
- A2 New `angle` quantity unit is rad; new `Vm` quantity unit is V. Rationale:
  consistency with the legacy `Kerr angle (rad)` header and conventional monitor-voltage
  notation.
- A3 `Vm` uses the same lock-in in-phase harmonic inputs as the angle computation
  of the active method (standard: LI1_in/LI2_in; harmonics: LI2_in/LI3_in) with
  `phim` / x0 shared from that method. Rationale: single source of inputs per
  method, no new channel plumbing.
- A4 Legacy run directories keep read compatibility only (re-runs emit new names).
  Stored artifacts are never rewritten. Rationale: preserves provenance immutability.
- A5 `pmoke kerr` remains as a deprecated alias that warns and delegates to
  `pmoke moke`. Rationale: script compatibility and a migration grace period.
- A6 Implementation lane follows the existing split: standard-method math lives in
  the embedded Python pytools (with Python unit tests), harmonics math lives in
  `pmoke-analysis-core` Rust (shared with WASM, with fixture tests). Rationale:
  preserves the WASM boundary and existing test ownership.
- A7 `Vm` CSV header notation is `Ch{N} Vm (V)`, mirroring the `angle` header
  `Ch{N} angle (rad)` renamed from legacy `Ch{N} Kerr angle (rad)`. Rationale:
  consistency with the existing header style.
- A8 `phim` for the standard method stays the hardcoded `0.92` default shared by
  the angle and `Vm` computations (no config promotion). Rationale: preserves
  legacy numerics exactly; promotion can be proposed separately with
  recalibration data.

## 4. Architecture

Affected layers: `pmoke` binary (config, acquisition, analysis, persistence,
display, export) -> `pmoke-analysis-core` (shared with WASM) ->
`pmoke-config-core` (shared config validation) -> `pmoke-web-wasm` (browser
boundary) -> Python bridge -> website (workers, visualization, MDX) ->
generated references. The policy of keeping hardware and Python dependencies out
of the WASM boundary is unchanged.

## 5. Functional requirements

### 5.1 Rename (moke)

- FR-1 Rename the config `[kerr]` section to `[moke]`. Meanings of `sensor`,
  `method`, and `factor` are unchanged (A1).
- FR-2 Bump the config schema from v5 to v6. Migrate v5 `[kerr]` by aliasing so it
  never trips `deny_unknown_fields` (D3).
- FR-3 Add CLI `pmoke moke`; keep `pmoke kerr` as a deprecated alias that warns
  and delegates (A5).
- FR-4 Rename run stage, artifact kind, and `published_through` value `"kerr"` to
  `"moke"`, including the `analysis/kerr/` directory → `analysis/moke/`. Keep
  read support for legacy run directories (A4).
- FR-5 Rename output files `kerr_results.csv` → `moke_results.csv` and
  `ch{N}_kerr.png` → `ch{N}_angle.png` (plus new `ch{N}_Vm.png` per FR-12).
  The `angle` column uses the `Ch{N} angle (rad)` header, renamed
  from legacy `Ch{N} Kerr angle (rad)` (D7/D8); the new `Vm` column uses `Ch{N} Vm (V)` (A7).
- FR-6 Rename modules, functions, and constants to moke/`angle` names with no
  remaining `kerr` outside compat shims (D5). Mapping includes: `kerr.rs` →
  `moke.rs`, `kerr_standard_analysis.*` → `moke_standard_analysis.*`,
  `kerr_harmonics_analysis.rs` → `moke_harmonics_analysis.rs`,
  `KerrStandardAnalyser` → `MokeStandardAnalyser`, `get_kerr_headers` →
  `get_moke_headers`, `KERR_HEADER` → `ANGLE_HEADER`, `KERR_NAME` → `MOKE_NAME`,
  `HarmonicsKerrOutput` → `HarmonicsMokeOutput`, `calculate_harmonics_kerr` →
  `calculate_harmonics_moke`, `Kerr`/`KerrType`/`KerrMethod` config types →
  `Moke`/`MokeType`/`MokeMethod`, `ValidationTarget::Kerr` →
  `ValidationTarget::Moke`, `resolver.kerr_csv()` → `resolver.moke_csv()`.
- FR-7 Rename the WASM API, synthetic settings, and tolerances
  (`kerr_angle_rad` → `angle_rad`, `kerr_rel` → `angle_rel`) while keeping the
  browser boundary free of hardware/Python dependencies.
- FR-8 Adapt embedded Python, plotting, and NPY export to the moke names.
- FR-9 Regenerate JA/EN documentation, generated references, and schema JSON,
  preserving locale parity (D6).

### 5.2 New quantities (angle, Vm)

- FR-10 Emit the `angle` (Kerr rotation, rad) column per D8 using the legacy
  computation of the active method (standard Python `calculate` / harmonics
  `calculate_harmonics_kerr`), unchanged.
- FR-11 Emit the `Vm` (monitor voltage, V) column per D8 from the same harmonic
  inputs (A3): standard from LI1_in/LI2_in with `phim`, harmonics from
  LI2_in/LI3_in with x0 = representative modulation depth.
- FR-12 Include the new columns in CSV, NPY, manifest column counts, plots, web
  visualization, and browser workers.
- FR-13 Apply numerical-analysis guards to both columns: finite validation,
  empty/short boundary cases, deterministic behavior, plus explicit guards for
  zero/near-zero Bessel denominators (`jn` has real zeros) and division-by-zero
  parity between the Rust and Python lanes. Add shared-core fixtures and
  tolerances for `Vm` (both methods) alongside the existing angle fixtures.
  Provide the Bessel `jn` function in the Rust lane within the dependency-light
  core boundary (existing workspace dependency preferred; any new dependency
  needs license/source review).
- FR-14 Validate new-quantity preconditions in config checking (channel presence,
  units, ranges) and fail before execution on violation.

### 5.3 Migration and safety

- FR-15 Add migration tests for v5 configs (`[kerr]` -> `[moke]` aliasing with
  values preserved).
- FR-16 Add read-regression tests for run directories containing legacy artifacts
  (A4).
- FR-17 This change must not alter safe-side acquisition/measurement behavior.
  Validate with dummy transports and isolated temp directories; live instrument
  operation needs separate authorization.

## 6. Non-functional requirements

- NFR-1 Do not regress the deterministic benchmark smoke test
  (`cargo bench -- --smoke`).
- NFR-2 Keep the check/test/clippy matrix across all features and transport
  profiles green.
- NFR-3 Preserve JA/EN parity, static export, the `/pmoke/` base path, and
  keyboard operability.
- NFR-4 Performance targets stay at current levels; quantify with measured updates
  when needed (pending measured update).

## 7. Milestones

- M0 Approval of these requirements (formulas frozen in D8). Freeze FR-1..FR-17
  and the NFRs.
- M1 Rename core (FR-1..FR-8) + migration tests (FR-15/FR-16). Rust validation
  matrix passes.
- M2 New quantities (FR-10..FR-14) + fixtures/tolerances. analysis-core/WASM
  validation passes.
- M3 Consistent web/Python/docs/generated updates (FR-9/FR-12). `pnpm check` +
  export verification passes.
- M4 Review 1 / Review 2, merge of the Issue-linked PR, reflection into the stable
  requirements.

## 8. Open questions (O)

- O1 Depth of legacy run-directory read compatibility (manifest only vs. CSV
  reinterpretation). Due: M1.
- O2 Sunset timeline for the `pmoke kerr` alias. Default: warn indefinitely (A5).
  Due: M4.

## 9. Review conclusion (2026-09-12 self-review)

Strengths:
1. Rename scope, compatibility policy, and update surfaces are frozen as D-items,
   leaving little room for implementation guesswork.
2. Audit gates (generated-output ownership, both locales, validation matrix) are
   built into the requirements.
3. Read compatibility for legacy assets is an explicit FR, preventing provenance
   breakage.

Limits and upcoming decision points:
1. `Vm` shares the standard lane's `phim` default (A8); any future promotion
   needs recalibration data.
2. Web worker/WASM display design for the added `Vm` column still needs separate
   browser-side work (M3).
3. The Python/Rust lane split (A6) means `Vm` needs two implementations plus
   parity tests; keep their zero-division behavior identical (FR-13). The Rust
   lane additionally has no Bessel `jn` today (analysis-core depends only on
   serde), so M2 must settle the provision route first.
