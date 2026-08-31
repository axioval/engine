# Independent adapters

Adapters are peers around the source-neutral engine. No adapter receives special privileges.

## OpenBIM

`axioval-openbim` maps IFC/OpenBIM objects, concepts, properties, classifications, relationships, placements and provenance to the IR. It does not depend on Axiolid and does not own geometry policy.

## Axiolid

`axioval-axiolid` supplies geometry evidence for any source capable of exposing Axiolid-compatible geometry handles. A proprietary CAD adapter can use it directly without importing OpenBIM or IFC.

## ICDD

`axioval-icdd` opens an ICDD package, dispatches member payloads to registered source decoders, maps linksets, and assembles a project. ICDD serialization types do not cross into the engine IR.

## Alternate geometry kernels

An OpenCascade or CGAL adapter may implement the same evidence traits in an external crate. Native/FFI code is never enabled by the default pure-Rust distribution.

## Conformance

Every source adapter must prove source-qualified identity, deterministic enumeration, provenance and strict malformed-data behavior. Every geometry adapter must prove exactness reporting, backend-failure propagation, transform/unit handling and cache isolation.
