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

## Component clearance

`axioval:capability.component-clearance` places a fixed box or cylinder beside each selected component in the component's own placement frame (see [Capabilities](./capabilities.md#free-space-around-components)). Which frame axis is the component's front is the rule's statement (`front_axis`), or the front the object-frame service states; it is never inferred from the placement axes, the shape or the type, and IFC states none. The volume starts at the component's outermost point on the stated side, measured as a directional extent, so it follows the body rather than the placement origin.

Every measured position is an interval, so the volume's position is too. The capability asks two questions: whether the union of every position the volume could take is clear (then it is clear wherever it is), and whether the part every position shares is obstructed (then it is obstructed wherever it is). Along a coordinate axis the intervals are points and one request answers both. Anything between is not evaluated.

A floating box, one that may slide sideways until it fits, needs the placement search below with a frame-offset domain and is not part of this capability yet (#83).

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

Frame-offset witnesses keep the anchor's axes, so a box searched in a frame-offset domain must be `Fixed` to those axes. Any other orientation is refused as `OrientationDomainConflict` when the request is built.

- `Found` carries one exact, scope-grounded placement frame. Supported domains additionally require exact evidence that the whole candidate base is supported by the requested source-qualified object at that exact frame and within the requested gap.
- `NoPlacement` requires complete exact search evidence.

A bounded or partial search cannot claim that no placement exists. Connected corridor requirements use [metric routing](./metric-routing.md) with an appropriate mobility profile rather than inventing a second path contract.

## Free-area bounds

`FreeAreaRequest` measures accessible area for a scope and mobility profile. `AreaInterval` stores conservative square-metre bounds. Minimum-area comparisons are three-valued:

- the lower bound meets the minimum: satisfied;
- the upper bound is below the minimum: violated;
- bounds straddle the minimum: indeterminate.

## Runtime boundary

`FreeSpaceServiceHandle` is registered in `ServiceRegistry`. It validates that every response is bound to the exact request. Missing geometry, unsupported composition, resource limits, and incomplete evidence return `FreeSpaceError`; none become clear, no-placement, or passing results.

The contract contains no IFC, STEP, ICDD, Axiolid, OpenCascade, CGAL, or vendor-specific type.
