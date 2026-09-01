# Axioval Engine implementation plan

## Goal

Publish a production-capable source-neutral pure-Rust rule engine, port reusable Solibri rule-kernel behavior, and rewire `projects/vendor/solibri` to consume it.

## Constraints

1. Engine IR is not IFC and never exposes STEP/entity handles.
2. OpenBIM, ICDD, and Axiolid are independent adapters.
3. Proprietary CAD sources may use Axiolid without OpenBIM.
4. Geometry is optional and replaceable; non-geometric rules run without a backend.
5. Rule packages are declarative and untrusted.
6. Results are deterministic, source-qualified, provenance-carrying, and fail closed.
7. Migration preserves Solibri behavior through dual-run parity gates.

## Current proven slices

- Release candidate `0.1.2`: immutable evidence sessions, strict IFC4 STEP import through published OpenBIM crates, exact direct occurrence/type property evidence, and evidence-preserving integer `not_equal`. Relationship completeness remains deliberately fail-closed. The Solibri property-comparison cutover consumes `0.1.0`; required-property consumer cutover remains open.

Unchecked workstream boxes below denote incomplete families, not an absence of all supporting primitives.

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

### 6. Solibri cutover

- [ ] CSET/spec conversion targets normalized Axioval contracts
- [ ] Checker executes Axioval plans
- [ ] CLI, Python and reporting consume Axioval reports
- [ ] IFC source uses OpenBIM adapter
- [ ] Geometry uses Axiolid adapter
- [ ] Duplicate engine/model/geometry ownership removed

### 7. Proof

- [ ] Full engine CI and architecture mutation tests
- [ ] Full Solibri CI-equivalent gates
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
