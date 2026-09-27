# `axioval-bcf`

BCF 2.1 and 3.0 issue archives from a `Report` and the `Project` it was computed over.

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
- A source- or project-scoped entry has no subject, so its topic has no
  viewpoint and no component, and nothing in it is `unanchored`.
- GUIDs are UUIDv5 over rule, GlobalIds and message, so they survive
  re-export. A scoped finding's key marks its scope kind but never names the
  source, which would change with the file name; `tests/export.rs` pins the
  object and rule-level GUIDs. Changing `NAMESPACE` or the key layout changes every GUID a user
  has ever received; treat both as a compatibility contract.
- A finding's `Priority` follows its severity; a not-evaluated outcome has
  none, because no severity was decided. Never invent one for it.
- Never read the clock or invent GUIDs; the caller supplies author and date.
- Cameras come only from host-supplied `Options::bounds`. A missing bound
  leaves the viewpoint without a camera (listed in `Export::unframed`),
  never a guessed one. The perspective viewpoint keeps the GUID of the
  camera-less viewpoint; the orthogonal one is derived beside it. Without
  bounds the archive is byte-identical to one written before cameras.
- 3.0 requires a camera on every viewpoint: refuse the export with
  `MissingCamera` rather than drop a viewpoint or write 2.1 instead.
- Visibility, colouring and clipping planes wait on `openbim-bcf` writing
  them; do not post-process the writer's XML.
- Run `cargo test -p axioval-bcf` and `cargo test -p axioval --all-features`;
  `tests/export.rs` is the contract and reads every archive back.
- A located entry's topic is labelled `Storey: <name>` and `Space: <name>`
  after the rule id (the place's id when unnamed). Labels never enter the
  GUID key: locating a finding must not change its GUID.
