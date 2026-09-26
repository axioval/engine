# `axioval-bcf`

BCF 2.1 issue archives from a `Report` and the `Project` it was computed over.

- Depends on `axioval-ir` and `openbim-bcf` only. Never the engine, never a
  source adapter, not even as a dev-dependency: a dev-dependency on an
  unpublished sibling breaks workspace package verification. The facade's
  `tests/ifc_bcf.rs` pins `IFC_GLOBAL_ID_SCHEME` to the adapter's
  `IFC_GLOBAL_ID` and runs IFC import through export.
- Every finding and, unless the host opts out, every not-evaluated outcome is a
  topic. Dropping not-evaluated outcomes by default would make an incomplete
  check read as a pass.
- A viewpoint selects the subject first. Never write a viewpoint of related
  objects alone: it would highlight the wrong element. An object without a
  GlobalId alias keeps its topic and is listed in `Export::unanchored`.
- GUIDs are UUIDv5 over rule, GlobalIds and message, so they survive
  re-export. Changing `NAMESPACE` or the key layout changes every GUID a user
  has ever received; treat both as a compatibility contract.
- Never read the clock or invent GUIDs; the caller supplies author and date.
- 2.1 only until a host can supply a camera, which 3.0 requires.
- Run `cargo test -p axioval-bcf` and `cargo test -p axioval --all-features`;
  `tests/export.rs` is the contract and reads every archive back.
