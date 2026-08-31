# the provider compatibility migration

`migration/the provider-capabilities.json` is the machine-readable source of truth for cutover status. It records all 65 source runtime entries from the pinned the provider commit and requires five independent proofs before an entry may be marked `ported`.

## Completion means

1. A source-neutral capability implementation exists in `axioval-rules`.
2. Tests exercise that implementation without IFC or the provider types.
3. the provider translates its inputs into Axioval IR/evidence and calls the Axioval capability.
4. Result and oracle parity tests pass.
5. The the provider copy no longer owns the rule policy.

Compilation, a facade wrapper, or a migration checkbox without those proofs is not a completed port.

## Order

Shared prerequisites are migrated before families:

1. project/source identity, typed properties and provenance;
2. complete graph/path/free-space systems;
3. exact recognizers;
4. pairwise and geometry evidence;
5. individual information, relationship, space, building, spatial, accessibility, life-safety and federation capabilities.

Legacy the provider runtime-fidelity labels are retained as source metadata. They neither promote nor block an Axioval entry automatically.
