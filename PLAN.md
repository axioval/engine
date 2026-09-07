# Axioval Engine implementation plan

## Goal

Publish a production-capable source-neutral pure-Rust rule engine, port reusable the provider rule-kernel behavior, and rewire `projects/vendor/the provider` to consume it.

## Constraints

1. Engine IR is not IFC and never exposes STEP/entity handles.
2. OpenBIM, ICDD, and Axiolid are independent adapters.
3. Proprietary CAD sources may use Axiolid without OpenBIM.
4. Geometry is optional and replaceable; non-geometric rules run without a backend.
5. Rule packages are declarative and untrusted.
6. Results are deterministic, source-qualified, provenance-carrying, and fail closed.
7. Migration preserves the provider behavior through dual-run parity gates.

## Current proven slices

- Release candidate `0.1.4`: source-neutral `LinearQuantityService` and the `ShelfCapacity` capability -- the first ADR 0004 decomposition, splitting a measured shelf run from the policy that judges it. Supersedes the yanked `0.1.3`, which leaked downstream-vendor references into published crates.
- Release `0.1.2`: immutable evidence sessions, strict IFC4 STEP import through published OpenBIM crates, exact direct occurrence/type property evidence, and evidence-preserving integer `not_equal`. Relationship completeness remains deliberately fail-closed. The the provider property-comparison cutover consumes `0.1.0`; required-property consumer cutover remains open.

Unchecked workstream boxes below denote incomplete families, not an absence of all supporting primitives.

## Extraction strategy (ADR 0002)

Three sidecar waves added `+9110` net LOC to `vendor/the provider` while moving 0 of
83 native rule implementations out of it. The per-family sidecar is
anti-convergent: it keeps both implementations alive by construction.

The blocker is the evidence seam, not rule policy. Legacy `GeometryProvider`
has 43 methods, **23 of which take a `spec::rule::*PlanSpec`** — one bespoke
evidence method per rule, so per-rule migration cost never amortizes.

Extraction therefore moves the contract and runtime first, and families follow
mechanically:

1. **Contract.** `crates/spec` (21,363 LOC, zero internal dependencies) becomes
   the engine's rule vocabulary next to `axioval-ir`'s package contract.
2. **Runtime.** Blocked on identity, not on code motion. Measured per file,
   `rules/src/engine` + `rules/src/engines` does **not** move wholesale:
   `compile.rs` (1,347 LOC) imports 68 concrete vendor rule types and is a
   catalog dispatch table; `glob.rs`/`java_pattern.rs` implement the provider
   `ConstraintUtils`/`Operators.MATCHES` compatibility and are vendor adapter
   code by ADR 0002's own ownership test. The remaining 4,000 LOC
   (`circulation/network.rs`, `selection/scope.rs`, `pairwise/mod.rs`,
   `result.rs`) is blocked on one thing: it is written against
   `EntityRef(pub u32)`, a per-model arena index, where ADR 0001 requires
   source-qualified opaque identity. The engine already owns that identity
   (`axioval_ir::ObjectId`, `Finding`, `Report`). Step 2 is therefore rewriting
   the runtime against neutral identity, not relocating files.
3. **Seam.** Not a relocation but a decomposition (ADR 0004). Of the 23
   plan-shaped provider methods, **17 return a verdict, not a measurement**:
   a `findings: Vec<Resolved*Finding>` list whose `kind` is a rule-specific
   enum (`CircleDoesNotFit`, `NarrowCorridor`). The compliance logic lives
   in the provider, not the rule: `StairRule` is 198 LOC that stringifies
   `finding.kind`, while `production/accessibility/stair.rs` is 1,645 LOC --
   8:1 policy in the provider. Each method splits into the measured
   quantity (behind a neutral service, source-specific) and the comparison
   against declared parameters (`axioval-rules`, source-neutral). The 6
   already-measurement-shaped methods go first to validate the seam.
4. **Families.** Rule policy ports against evidence that already exists.
5. **Vendor.** `vendor/the provider` keeps only CSET/SMC byte formats, authoring,
   CLI and Python — one application on the engine, not the engine's home.

Enforced by `scripts/architecture.py`: a `*Service` trait may not take a
`*PlanSpec` or be named after a rule in the migration ledger. The gate is
mutation-proven against injected leaks in a real `axioval-engine` service trait
(`7/7` killed, covering verb-prefix laundering, comment-brace shielding, type
aliasing, generic methods and ledger schema drift).

No new `families/<name>/axioval.rs` sidecars. The three that exist are removed
when their families move.

## Workstreams and completion gates

### 1. Repository and publication

- [x] Cargo workspace and ownership boundaries
- [x] README, progressive `AGENTS.md`, plan and docs structure
- [ ] CI, GitHub Pages and release automation pass remotely

### 2. Contracts and runtime

- [ ] Project/source/view/object identity and semantic value IR
- [ ] Selectors, relationships, provenance and typed evidence
- [ ] Normalized Axioval package loader and strict binder
- [ ] Trusted capability registry and deterministic compiler
- [ ] Runtime, budgets, cancellation, findings and reports

### 3. Independent adapters

- [ ] OpenBIM semantic adapter
  - [x] Strict IFC4 STEP session and exact direct occurrence/type property source
  - [ ] Exact relationship, classification, placement, and remaining semantic sources
- [ ] Axiolid geometry adapter usable by any source
- [ ] ICDD project assembly adapter
- [ ] Adapter conformance kit and mock alternate geometry backend

### 4. Shared rule systems

- [ ] Selection and quantifiers
- [ ] Pairwise/spatial candidate engine
- [ ] Space/path/free-space graph
- [ ] Exact recognizers and typed property sources
- [ ] Evidence caches and exactness propagation

### 5. Capability migration

- [ ] Information/property families
- [ ] Relationship and space families
- [ ] Building and spatial families
- [ ] Accessibility and circulation families
- [ ] Life-safety families
- [ ] Federation/comparison families

### 6. the provider cutover

- [ ] CSET/spec conversion targets normalized Axioval contracts
- [ ] Checker executes Axioval plans
- [ ] CLI, Python and reporting consume Axioval reports
- [ ] IFC source uses OpenBIM adapter
- [ ] Geometry uses Axiolid adapter
- [ ] Duplicate engine/model/geometry ownership removed

### 7. Proof

- [ ] Full engine CI and architecture mutation tests
- [ ] Full the provider CI-equivalent gates
- [ ] Per-capability oracle/parity ledger
- [ ] Benchmarks with baselines
- [ ] Clean-clone documentation and examples
- [ ] Independent final review

## Rollback

Migration is capability-scoped. Each wave is an atomic scoped commit and can be
reverted independently. A declaration recognized as supported by the Axioval
facade never silently falls back to legacy evaluation: unavailable or incomplete
session evidence is terminal. Unsupported declaration shapes remain explicitly
outside that cutover until separately implemented and certified.
