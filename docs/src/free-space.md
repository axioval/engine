# Free space and clearance

Free-space computation is a typed host service. The engine owns validated source-neutral requests and evidence; Axiolid or another trusted geometry provider owns rasterization, CSG, collision tests, search, and spatial indexes.

## Fixed clearance volumes

`ClearanceRequest` combines:

- an object-grounded `MetricFrame` in canonical metres;
- orthonormal right, forward, and up directions;
- a validated box or cylinder;
- a deterministic source-qualified obstacle candidate set selected by the trusted rule capability.

The frame's origin is the centre of the volume's base: a box extends half its width either side along `right`, half its depth either side along `forward`, and its height up from the origin; a cylinder is centred on the origin. Semantic filtering stays outside geometry: the provider evaluates exactly the supplied candidate objects. This represents directional component clearance without leaking IFC placements, meshes, B-reps, or kernel types into the engine.

The result is asymmetric:

- `Obstructed` needs one exact, non-empty obstruction witness;
- `Clear` needs exact and complete obstacle coverage for the request.

Partial geometry therefore cannot produce a false clear result.

The Axiolid adapter measures a box as the exact rectangle along the frame's right and forward axes, swept over its height, and refuses a tilted frame. It bounds a cylinder from both sides with prisms over inscribed and circumscribed 64-gons. An obstacle meeting the inscribed prism obstructs; the volume is clear only when every obstacle misses the circumscribed one. An obstacle between the two (within about 0.12 % of the radius) refuses the request rather than being guessed either way.

An obstacle obstructs only when its solid and the volume share interior. Its height range and plan outline are never tested separately: an L-shaped body with a low foot under the volume and a tall column beside it overlaps the volume in both, yet never enters it. The adapter tests the solid against the prism shrunk by 1 µm on every side, so a body resting on the base, standing against a side or touching the top leaves the volume clear:

- a triangle of the obstacle meets the shrunk prism: the part of the triangle inside the height band is a convex polygon, which meets the prism exactly when it meets the prism's plan polygon, a separating-axis test between two convex polygons. This also catches an obstacle wholly inside the volume;
- otherwise the prism lies wholly inside or wholly outside the obstacle, and the winding number at the volume's centre, far from every triangle, tells which.

Only a closed, consistently and outward wound mesh bounds a solid. An obstacle whose mesh box reaches the shrunk prism must be one, or the request is refused; one whose box misses it is clear whatever its mesh.

## Containment in scopes

`ContainmentRequest` asks whether the same volume's plan footprint lies inside the union of the footprints of source-qualified scopes, such as the spaces a component stands in (merged spaces are several scopes). Only the plan is compared. The scopes are the rule's selection; with none, nothing covers the footprint.

- `Inside` claims that no part of the footprint of positive area lies outside every scope;
- `Outside` claims that some part does.

Both carry exact evidence bound to the request. The Axiolid adapter takes the overlay difference of the footprint's bounds less the scopes' projected triangles, so the part outside is measured directly rather than as a small difference of two large areas: the outer bound left with nothing is `Inside`, the inner bound left with something is `Outside`, and a cylinder whose band straddles a scope boundary is refused. A scope without a body, unmeasured or tessellated is refused. A service that does not implement containment refuses by default, never answering either way.

## Support coverage

`SupportCoverageRequest` asks whether the tops of source-qualified supports hold the same volume's plan footprint: the upward-facing surfaces of their bodies, clipped to an elevation band (`from_metres` to `to_metres`), such as a slab or landing near the floor under a door's clear area. Only the plan and the band are compared. The supports are the rule's selection; with none, nothing holds the footprint, and a band that is not finite or reversed is refused.

- `Supported` claims that no part of the footprint of positive area lies outside the tops in the band;
- `Unsupported` claims that some part does.

Both carry exact evidence bound to the request (`SupportCoverageEvidence`). The Axiolid adapter takes every triangle of a support facing up, clips it to the band and projects it, then measures the overlay difference of the footprint's bounds less those tops, as for containment: the outer bound left with nothing is `Supported`, the inner bound left with something is `Unsupported`, and a cylinder whose band straddles a top's edge is refused. A support without a body, unmeasured or tessellated is refused. `FreeSpaceService::assess_support_coverage` refuses by default, never answering either way, and the handle refuses an answer to another request. `component-clearance` asks it twice: the union of the volume's positions against sure supports within the band every possible base shares (supported), and their common part against every possible support within the band any base reaches (unsupported).

## Component clearance


`axioval:capability.component-clearance` places a fixed box or cylinder beside each selected component in the component's own placement frame (see [Capabilities](./capabilities.md#free-space-around-components)). Which frame axis is the component's front is the rule's statement (`front_axis`), or the front the object-frame service states; it is never inferred from the placement axes, the shape or the type, and IFC states none. A door's swing side is stated, though: `front_axis` `swing` takes the direction its hinged leaves open towards from its leaves, with world up, so a door placed upside down is measured upright. With `front_axis` `against-wall` the front is derived from the walls beside the sides of the footprint's least-area rectangle: the side surely nearest a wall is the back, a tie is not decided, and the derived front is inexact evidence naming the wall. The volume starts at the component's outermost point on the stated side (or its midline, with `depth_from` `midline`), measured as directional extents, so it follows the body rather than the placement origin. A width, depth or height sized from the component is measured the same way, as an interval, and the volume is kept as the intervals its edges and its base and top are known to: its union is bounded by the outermost ends of those intervals, its common part by the innermost.

Every measured position is an interval, so the volume's position is too. The capability asks two questions: whether the union of every position the volume could take is clear (then it is clear wherever it is), and whether the part every position shares is obstructed (then it is obstructed wherever it is). Along a coordinate axis the intervals are points and one request answers both. Anything between is not evaluated.

A floating volume, one that may slide sideways or away from the component until it fits, is a placement search below instead: the scope is the first space `space_path` reaches, merged with the others; the domain is a frame-offset domain anchored on the component, with the side's plan axes, the slide interval across (`slide_from`, `slide_to`), the slide interval away from it (`depth_slide_from`, `depth_slide_to`) and the base on the scope's floor; the box is fixed to the anchor's axes; the band runs from the base up to the top above the floor. The union and common part become the volume grown and shrunk by the position interval. A found witness is a volume free wherever the component is (along an axis it does not slide on, the grown volume is also searched 1 mm larger each way within 1 mm of the line, so the search is not confined to a line, which has no area); a proof of no placement for the shrunk volume is a volume obstructed at every offset. A maximum size is asked as the volume one tolerance larger in each dimension, fixed or floating, with the answers swapped: free is the finding.

Accessible routes use the same search for passing spaces: frame-offset domains anchored on each segment of the walked route, one tile of the route at a time (see [Capabilities](./capabilities.md#passing-spaces)).

## Placement search

`PlacementRequest` asks whether a box or cylinder can fit in a source-qualified search scope. Its `PlacementShape` states which rotations count, because "fits" depends on it: a box that fits only diagonally has no placement along a fixed frame but has one at some angle.

- A box carries a `PlacementOrientation`. `Fixed(frame)` binds the box width to the frame's right axis and its depth to the forward axis; only the axes are binding, not the frame origin. The frame comes from the rule's context, such as a door leaf, a fixture or a declared room axis, and never defaults silently to the model's world axes. `Any` quantifies over every rotation about the vertical axis.
- A cylinder is rotation-invariant and carries no orientation.

A `Found` witness for a fixed orientation must use exactly the fixed axes; any other frame is rejected as `PlacementOrientationMismatch`. Evidence for a fixed orientation answers only that orientation: "no 1.80 × 1.80 m area aligned with the corridor" does not mean "no area at any angle".

A `PlacementDomain` makes the admissible candidate set explicit:

- `Unconstrained` preserves generic searches without implying support;
- `Supported` requires the whole candidate base on a source-qualified support object within a maximum gap;
- `FrameOffsets` limits right/forward/up translation in an anchor frame;
- `SupportedFrameOffsets` requires both support and anchor-relative bounds.

Frame-offset witnesses keep the anchor's axes, so a box searched in a frame-offset domain must be `Fixed` to those axes. Any other orientation is refused as `OrientationDomainConflict` when the request is built. The anchor may be grounded on another object than the scope, such as the door or fixture whose frame the object-frame service states, with the front axis the rule's statement; the witness is still grounded on the scope. `FrameOffsetPlacement::contains_frame` is the test every witness must pass, and backends may filter with it.

Two request options shape the search:

- `with_band(ElevationBand)` states the band, from and to metres above the scope's floor, in which obstacles count. Without it the band runs from the floor up by the shape's height (`effective_band`). A band must start at or above the floor and end above its start.
- `with_merged_scopes` searches the union of the scope and further scopes, such as the spaces of one group. A merged scope may be neither the scope nor an obstacle (`MergedScopeConflict`).

- `Found` carries one exact, scope-grounded placement frame. Supported domains additionally require exact evidence that the whole candidate base is supported by the requested source-qualified object at that exact frame and within the requested gap.
- `NoPlacement` requires complete exact search evidence.

A bounded or partial search cannot claim that no placement exists.

### The Axiolid placement search

The Axiolid adapter decides placement in the scope's configuration space: the set of centres at which the shape lies in the scope footprint and meets no obstacle in its height band. That set is the scope eroded by the shape, minus every obstacle footprint dilated by the reflected shape. A point in it is a witness; an empty set proves that no placement exists.

The height band runs from the scope's floor (the bottom of its body) up by the shape's height, or over the request's elevation band, and is open at both ends: a body resting on the floor under the shape or starting exactly at its top does not block it. As in clearance, an obstacle counts only by the part of its solid inside the band, never by its height range and plan outline taken apart: its band footprint is its boundary clipped to the band, plus, for a body reaching down to the floor, its section just above the floor. So an L-shaped body with a column along a wall and an arm overhead blocks only the column's strip. Clipping planar triangles to horizontal planes is exact. A body reaching down to the floor must be closed and consistently wound, or the search refuses. An obstacle without a measured body, a tessellated scope, or a tessellated obstacle within reach refuses too.

| Shape | Witness (`Found`) | Proof of absence (`NoPlacement`) |
|---|---|---|
| Box, `Fixed` | a centre in the exact configuration space, re-verified by direct overlap | the configuration space is empty even for the box shrunk by 1 µm per side |
| Box, `Any` | a fixed-orientation witness at the middle of an angle interval | every interval is empty for the middle box shrunk by the half-diagonal times the half-interval |
| Cylinder | a centre in the configuration space built from inner disc approximations, which lie inside the exact one | the configuration space built from outer disc approximations, which contain the exact one, is empty for a radius 1 µm smaller |

Anything in between refuses: a fit by contact, a circle whose fit lies inside the disc approximation band (about 0.12 % of the radius), or an angle search whose budget (512 fixed-orientation checks over half a turn, a quarter turn for a square) runs out before every interval is decided. The shrink margins keep rounding from turning a fit by contact into a false proof of absence.

The search runs on the scope's own floor: `Unconstrained` and `Supported` by the scope itself, each also with frame offsets. Other supports are refused until their support rules are written down. With merged scopes the footprint searched is the union of all of them; they must share the scope's floor (within 1 nm), every one is checked for tessellation like the scope, and only an unsupported domain is answered, since no single support holds a base spanning several of them. A `Fixed` frame must be upright. The witness frame stands at the floor elevation and uses the requested axes for `Fixed`, the anchor's for a cylinder in a frame-offset domain, or the found angle for `Any`. Evidence cites the scope's own source; its locator names the merged scopes.

A frame-offset domain is searched exactly for fixed orientations: the configuration space is intersected with the box of centres the right and forward offsets allow. The anchor must be exactly upright, and the scope's floor must lie within the up offsets, or the search refuses rather than proving the domain empty. A witness comes from the offset box as computed and must also pass `contains_frame`; a proof of absence needs the configuration space of the shrunk shape to miss even the box grown by 1 µm, so rounding the box's corners cannot hide a centre on its edge. A box pinned to one offset has no area: the configuration space's candidates are moved onto it and re-verified, and its absence is proven with the grown box. For a cylinder the same intersection applies to the inner and outer configuration spaces.

Witness candidates are the free region's centroids and the middle of every interval a line halfway between two consecutive vertex heights cuts from it, so a free region with a hole (a room around a column) is never missed because its centroids fall into the hole.

A free corridor width (a region eroded by half the width that still connects two sides) is not searched here: the sides are not part of a placement request. Within one space, a [circulation map](#circulation-maps) answers it between entrances and components; across spaces, corridor widths use metric routing.

Connected corridor requirements use [metric routing](./metric-routing.md) with an appropriate mobility profile rather than inventing a second path contract.

## Circulation maps

`FreeSpaceService::map_circulation` answers a `CirculationRequest`: a space, its entrances, its components, the obstacles, a path width, the height the path must be free to and a tolerance. Entrances are never obstacles and the space is none of the three; the constructor removes them. A service that does not map circulation refuses by default. The `CirculationMap` is a measurement:

- **Pieces**: the connected parts of a region inside the space's free area (its floor less what the obstacles occupy in the band from the floor up by the height) eroded by half the width. Every point of a piece is the centre of a free disc as wide as the path, so a path runs between any two points of one piece.
- **Possible pieces**: the connected parts of a region containing that erosion. A path between two different possible pieces does not exist.
- **Contacts**: for each entrance and component (the request's subjects, sorted), the pieces proven to come within half the width plus the tolerance of its plan footprint, each with the skeleton node nearest to it, and the possible pieces that may.
- **Skeleton**: nodes (`CirculationNode`: a point on the floor, a kind by its number of neighbours, `End`, `Path`, `Junction` or `Isolated`, its piece, and bounds on its distance to the free area's boundary, the half width there) and edges within one piece. Node positions are approximate: that a node lies in its piece and its half width are proven, how close it is to the true medial axis is not. The map states the boundary sample spacing it was built with.
- **Unmapped pieces**: pieces whose skeleton could not be built, with why; their contacts still stand, but where their paths end is unknown.

`CirculationMap::try_new` checks the map's shape: nodes in pieces, edges within one piece, kinds matching the neighbours, one contact per subject naming only pieces and nodes that are there, and exact evidence. The handle rejects a map of another request.

The Axiolid adapter builds the free area as the placement search's scene does. The pieces are the polygons of the scope eroded by `Region::erode_inner` less the obstacles grown by `dilate_outer`; the possible pieces the polygons of `erode_outer` less `dilate_inner`, for half the width less 1 µm, so a gap exactly as wide as the path keeps a sliver and is neither proven nor ruled out. A contact is proven when a piece meets the subject's footprint grown by `dilate_inner`, possible when a possible piece meets it grown by `dilate_outer`. The skeleton is `axiolid_route::skeleton` of each piece with pruning factor 1.5 and a boundary spacing of a tenth of the width (at most 4000 samples per piece); a disconnected skeleton is built once more with the spacing down to half the narrowest width of the erosion. Half widths are distances to the free area's boundary widened by the overlay's grid snapping. A subject without a body, unmeasured or tessellated refuses the map.

A piece whose skeleton the kernel refuses is unmapped, never guessed.

Door swings are not subtracted: the map counts only what the obstacles occupy, although a source may state door leaves (`ObjectFrameService::leaves`).

## Free-area bounds

`FreeAreaRequest` measures accessible area for a scope and mobility profile. `AreaInterval` stores conservative square-metre bounds. Minimum-area comparisons are three-valued:

- the lower bound meets the minimum: satisfied;
- the upper bound is below the minimum: violated;
- bounds straddle the minimum: indeterminate.

## Runtime boundary

`FreeSpaceServiceHandle` is registered in `ServiceRegistry`. It validates that every response is bound to the exact request. Missing geometry, unsupported composition, resource limits, and incomplete evidence return `FreeSpaceError`; none become clear, no-placement, or passing results.

The contract contains no IFC, STEP, ICDD, Axiolid, OpenCascade, CGAL, or vendor-specific type.
