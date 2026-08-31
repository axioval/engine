# Free space and clearance

Free-space computation is a typed host service. The engine owns validated source-neutral requests and evidence; Axiolid or another trusted geometry provider owns rasterization, CSG, collision tests, search, and spatial indexes.

## Fixed clearance volumes

`ClearanceRequest` combines:

- an object-grounded `MetricFrame` in canonical metres;
- orthonormal right, forward, and up directions;
- a validated box or cylinder;
- a deterministic source-qualified obstacle candidate set selected by the trusted rule capability.

Semantic filtering stays outside geometry: the provider evaluates exactly the supplied candidate objects. This represents directional component clearance without leaking IFC placements, meshes, B-reps, or kernel types into the engine.

The result is asymmetric:

- `Obstructed` needs one exact, non-empty obstruction witness;
- `Clear` needs exact and complete obstacle coverage for the request.

Partial geometry therefore cannot produce a false clear result.

## Placement search

`PlacementRequest` asks whether a box or cylinder can fit anywhere in a source-qualified scope. This supports free-floor-space requirements that ask for a free rectangle or turning circle rather than only a total area.

- `Found` carries one exact, scope-grounded placement frame.
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
