# Migration from Solibri

A box is checked only when the Axioval implementation is the production owner, Solibri consumes it, and parity evidence passes.

## Shared runtime

- [ ] Normalized package contract and strict binder
- [ ] Capability registry/compiler
- [ ] Deterministic runtime/reporting
- [ ] Source/evidence sessions and caches
- [ ] Selection engine and quantifiers
- [ ] Pairwise/spatial engine
- [ ] Circulation/path/free-space graph
- [ ] Exact recognizers
- [ ] Typed property sources

## Capability families

- [ ] Information and property
- [ ] Relationship and assignment
- [ ] Space and topology
- [ ] Spatial clearance/distance
- [ ] Building
- [ ] Accessibility
- [ ] Life safety
- [ ] Federation and comparison

## Host rewiring

- [ ] `spec` normalizes to Axioval contracts
- [ ] `checker` executes Axioval plans
- [ ] CLI and Python use Axioval reports
- [ ] IFC parsing/modeling uses `openbimrs/ifc`
- [ ] Geometry evidence uses `axioval-axiolid`
- [ ] ICDD uses `openbimrs/icdd` plus `axioval-icdd`
- [ ] BCF/report adapters consume generic findings
- [ ] Duplicate engine/model/geometry ownership removed

## Required evidence per row

Record the Axioval tests, Solibri tests, oracle corpus, discrepancy count, performance measurement, cutover commit and rollback switch. Unsupported behavior remains unchecked and documented; it is never represented by an empty module or unconditional success.
