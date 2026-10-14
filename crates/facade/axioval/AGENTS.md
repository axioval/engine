# `axioval`

Batteries-included facade and feature-gated adapter re-exports.

Default features remain source- and backend-neutral. OpenBIM, ICDD, and Axiolid are independent opt-in features; never introduce a combined adapter feature or dependency edge.

The one join is a host-layer crate: `ifc-geometry` re-exports `axioval-host-ifc-axiolid` as `ifc_geometry` and enables `ifc` (to import the sessions it attaches to), never `axiolid`. A feature may enable a host crate, but never both adapters itself; `scripts/architecture.py` (`joined_sides`) fails any feature whose dependencies join the IFC and the Axiolid side outside `HOST_CRATES`.
