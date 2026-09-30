# `axioval-bcf-snapshot`

Illustrative PNG snapshots of BCF viewpoints, drawn from tessellated meshes
through the sink's `SnapshotRenderer` hook.

- A snapshot is illustrative, never evidence. Never derive a measurement,
  a finding or an outcome from it; the sink marks every topic carrying one
  with `SNAPSHOT_NOTE`.
- Depends on `axioval-bcf`, `axioval-ir`, `openbim-bcf` (camera types) and
  `flate2` only. Never a source adapter, an IFC type or a geometry kernel:
  the host hands in meshes (`Mesh`) in model coordinates, in metres.
- Pure Rust, deterministic: pixel-centre sampling, a fixed arithmetic
  order, unfiltered rows, deflate level 6. Same input, same bytes, for a
  given build.
- Decline (`None`) rather than draw a misleading image: no mesh for the
  subject, or a degenerate camera. The sink lists the subject in
  `Export::unrendered`.
- The subject and related objects are drawn in the viewpoint's colours
  (the sink's defaults when the archive writes none), everything else in
  `CONTEXT_COLOR` unless the viewpoint isolates; clipping planes cut the
  triangles before projection.
- Run `cargo test -p axioval-bcf-snapshot`; `tests/snapshot.rs` renders a
  clash topic through the sink and reads the PNG back from the archive.
