# `axioval-axiolid`

Geometry evidence for any source, measured with the Axiolid kernel.

- `src/geometry.rs` holds `AxiolidGeometry`, the host-supplied mesh store shared
  by every service here. Doorway counts live here too: a mesh does not say which
  wall segments are openings, so the host declares them.
- `src/guard.rs` implements `GuardService`: barriers, landings and climbing
  aids around a walking surface's edge. Proximity is footprint-to-footprint,
  never vertex-to-vertex.
- `src/free_space.rs` implements `FreeSpaceService` for clearance and free
  area. `find_placement` refuses: its `NoPlacement` arm asserts an exhaustive
  search this adapter cannot perform.
- `src/space.rs` implements `SpaceService`: seven independent space
  measurements over storey-assigned, role-tagged geometry.
- `src/envelope_membership.rs` derives envelope membership: an object bounds
  the envelope when its plan footprint meets a declared bounding space. The
  declared set is plain `ObjectId` data, so this stays a geometry adapter.
- `src/planar.rs` (internal) holds the plan-projection helpers shared by the
  services; `src/geometry.rs` holds the mesh store and triangle vocabulary.
- `src/linear_quantity.rs` implements `LinearQuantityService`, measuring shelf
  running length from footprint, real ceiling height and doorways. It reports an
  upper bound, never an exact value, because the footprint bounds a
  non-rectangular room rather than describing it.
- `src/contact.rs` implements the engine's `ContactService` over `axiolid-mesh`,
  `axiolid-measure` and `axiolid-overlay`. Hosts register a `TriMesh` per
  `ObjectId`; no IFC types appear anywhere in this crate.
- **Contact area is measured in plan**, by intersecting the projected triangle
  soups (`axiolid-overlay`). Do not go back to per-triangle counting: a triangle
  spanning the whole face counts entirely as soon as any part of it approaches,
  which reported a half-supported slab as fully supported.
- **Nearest separation is a 3D question** and stays on `closest_points_on_triangles`;
  plan overlap alone would count anything passing overhead as contact.
- The area clamp against the subject's own face area is load-bearing: independent
  counterparts are measured independently and can double-count a shared region.
- `src/lib.rs` keeps the source-scoping contracts and the in-memory conformance
  double. `UnavailableGeometryBackend` remains the explicit "no kernel linked"
  placeholder.
- This crate must remain independent of `axioval-ifc`; proprietary CAD sources are
  first-class inputs.
- Run `cargo test -p axioval-axiolid` plus strict clippy. `tests/contact.rs`
  measures real meshes through the published kernel, not a stub;
  `tests/conformance.rs` protects source scoping.
- Mutation-check decision logic with `scratch/mutate_contact.py` before changing
  how contact is measured.

## Pitfall

`axiolid-measure` 0.1.0 does **not** export `mesh_distance`; that exists only in
unpublished local work. Fold separations from `closest_points_on_triangles`
rather than depending on an API the registry does not have.
