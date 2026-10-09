# `axioval-axiolid`

Geometry evidence for any source, measured with the Axiolid kernel.

- `src/geometry.rs` holds `AxiolidGeometry`, the host-supplied mesh store shared
  by every service here. `with_extent_bounds` takes host-certified bounds
  on a tessellated body's extent (the CLI passes
  `ExactBoundary::extent_bounds`: each item's subject in closed form
  outside, the built vertices less the body's widening inside);
  `vertical_bounds` reads them, never for an exact mesh. It also builds a whole's body from its registered
  parts (`compose`, `ComposedBody`, `with_composed_body`): the parts' meshes
  side by side (unwelded, so each stays a closed solid), exact only when every
  part is, else declared within the largest part deviation, with
  `ExactBody::union` where every meshed part has an exact body. It fails
  closed on the first unmeasured or stateless part in identity order and
  never treats an unknown part as bodiless. `shares_body` (a common piece)
  is what `AxiolidProximityService::shares_body` answers, and `body_note`
  is the locator suffix proximity, distance, volume, triangle-count and
  vertical-extent evidence carry: `;union:<whole>=<n>-parts` for a whole,
  and `;applied-openings:<host>=<opening>+...` for a host whose registered
  body already carries those openings' voids (`with_applied_openings`; the
  host states which and why, nothing is subtracted here), and `;whole-openings:<body>=<opening>+...` for a body the host subtracted wholes' openings from (`with_whole_openings`: a part cut by its whole's opening and each whole holding it; again nothing is subtracted here). Volumes of a
  whole are bounded piece by piece in `proximity.rs` (`union_volume`,
  `composed_intersection`), never on the concatenated mesh, which would
  count overlaps twice. A whole's `union_volume` (a boolean per two of its
  pieces) is taken once per service and cached, never per pair it is in. `tests/composed_bodies.rs`,
  `tests/applied_openings.rs` and `tests/exact_bodies.rs` pin it.
- `src/exact_boundary.rs` builds a body's exact solid from the Axiolid
  geometry graph a host meshes (`exact_boundary`), and
  `AxiolidGeometry::check_exact_boundary` compares its extent with the
  registered mesh. Instance placements (single-child collections looked
  through) are composed into one transform and applied once with
  `ExactBRep::transformed` (axiolid-brep 0.3.3, axiolid/kernel#223) to the
  solid built in its own coordinates: an extrusion
  (`extrude_profile_exact`; against the profile normal it is built along
  it and mirrored in the profile plane, a hollow circle goes as its
  contour), a revolution (`revolve_profile_exact`) or a disk swept along
  one segment or arc (`swept_disk_along_*_exact`; the directrix read by
  axiolid-mesh-compile's public `exact_directrix`, 0.3.8,
  axiolid/kernel#230, as its compilers read it; never re-implement that
  reading here, and refuse an `ExactDirectrix` form not yet known).
  An oblique extrusion of a profile with arcs is built by the kernel with
  exact elliptical-cylinder walls (axiolid-construct 0.3.17,
  axiolid/kernel#280); an oblique ellipse and a direction in the profile
  plane are refused by the kernel by name (#281). Never guard extrusions
  here before the kernel is asked (#317).
  Every other node, a scale or shear, and every kernel refusal
  refuse with the reason; never approximate a placement, an ellipse or a
  sweep. The extent is computed in closed form from the construction and
  the transform (support functions of the section, cosine ranges over
  revolution and arc angles); keep it so, never from sampling, and keep
  the reflection of a downward extrusion in both the solid's placement and
  its extent. `tests/exact_boundary.rs` checks every family against the
  kernel's own mesh of the same graph, the exact volume and certified
  distances against closed forms.
  A difference (openings, `IfcBooleanResult`) or a clip by a half-space,
  bounded or not (`IfcBooleanClippingResult`, axiolid/kernel#234), is built
  by `ReferenceExactCompiler::compile_exact_with_report`
  (axiolid-mesh-compile 0.3.13), first with `Tolerance::ZERO` and only on a
  refusal with `Tolerance::METRE`. Keep that fallback: a contact that holds
  only up to rounding (a flush window in a turned wall, a round hole
  touching a turned beam's flange, #243/#249) needs a reading within
  tolerance, which the kernel refuses at `Tolerance::ZERO`
  (`BooleanError::ToleranceExceeded`, axiolid-brep-boolean 0.1.5, #251).
  An empty `BooleanReport` (#236) is the exact boolean of the
  operands as given: the body is exact. A non-empty one perturbs the
  `ExactBody` by the reported linear magnitude plus the angular one over
  its extent, never less than its rounding; non-empty is the test, never
  a magnitude above zero (a reading within tolerance may report a zero
  magnitude), so `Decided::perturbed` carries
  `!report.is_exact()` and `ExactBody::is_exact` reads that flag, not the
  perturbation. Never register such a body as exact, never drop the
  perturbation from a certified distance (`certified` adds it), never
  certify a plan overlap or a surface distance on it. Either way the
  boolean merges constructed points closer than its rounding floor
  unreported, which the compiler's report states
  (`BooleanReport::rounding_floor`, axiolid/kernel#244, mesh-compile
  0.3.12: `2^-40` of the largest coordinate of any boolean's or clip's
  operands beneath the body, as the kernel reads it). Never mirror the
  kernel's factor or extent reading here; read the floor. The body
  carries booleans times that floor (the per-boolean count is ours: each
  boolean of a chain may merge points within it) as
  `ExactBody::rounding_metres`, which widens every certified
  distance (`widening_metres`) and plan-overlap gap but leaves the body
  exact. It is far above the `16 ε` witness rounding, so never drop it. A
  boolean item's extent is its edges' (`edge_extent`: faces on planes and
  cylinders take their extremes on edges, arcs by cosine ranges), its
  innermost subject's (`Shape::Placed`) only where another surface or edge
  family appears, so a clipped wall's extent is its ridge's. A collection
  of several solids becomes one item each
  in the body's frame, the placement above the collection kept apart
  (`ExactBody::placement`, axiolid/kernel#229); one item is placed in the
  world, as the one-solid measurements need. `tests/exact_bodies.rs` pins
  exact openings (a pipe through a window), a flush window in a turned
  wall (perturbed by its report), roof clips with a round window (a pipe
  over the slope), a bounded half-space, a column on its footing in space
  and in plan, a column over a shaft's edge, an IPE beam less a web hole
  (exact at no tolerance, #250), a root-filleted I-beam whose web hole
  touches its flange (exact under axis matrices, perturbed when turned; a
  hole half a micrometre off the flange is decided exactly at no
  tolerance, where a reading within tolerance would be refused by name)
  and the fallbacks.
- `src/guard.rs` implements `GuardService`: barriers, landings and climbing
  aids around a walking surface's edge. Proximity is footprint-to-footprint,
  never vertex-to-vertex.
- `src/free_space.rs` implements `FreeSpaceService` for clearance, placement
  and free area. `find_placement` builds the scene (scope footprint, and each
  obstacle's `walkable::band_footprint` in the open band from the scope's
  floor up by the shape's height, never its whole projection; built once
  per service and band, cached by obstacle and the band limits' bits,
  since spaces on one floor share obstacles and a large one's footprint
  can take the overlay minutes) and hands it to
  `src/placement.rs`: the configuration space by Minkowski erosion and sum
  with the convex rectangle, one-sided disc morphology for circles, angle
  interval search for `Any`. Witnesses are re-verified by direct overlap;
  `NoPlacement` needs the shape shrunk by `KNIFE_EDGE_METRES` (plus the
  rotation margin) to fit nowhere, so a fit by contact refuses. Never drop a
  margin or return `NoPlacement` from an undecided interval. Only the scope's
  own floor is searched (merged scopes must share it, and then no support
  is answered); other supports refuse. Obstacles are clipped to the
  request's `effective_band`. A frame-offset domain is a `Window`: witnesses
  from the box as computed and filtered by the contract's `contains_frame`,
  absence proven against the box grown by `KNIFE_EDGE_METRES`; never prove
  absence against the ungrown box. Witness candidates include slab
  midpoints, so a free region with a hole always offers interior points.
  Swept door sectors (`swept_rings`, `SWEEP_SEGMENTS` chords per quarter
  turn) join the scene twice: circumscribed in `Scene::obstacles`, which
  witnesses avoid, inscribed in `Scene::sure`, against which absence is
  proven; never prove absence against `obstacles` or find a witness
  against `sure`.
  An entrance reach (`Scene::reach`, built by `circulation.rs`'s
  `reached` as a map's contacts are) limits centres to where the shape
  meets the surely reached pieces (Minkowski sum, `dilate_inner` for a
  circle), re-verified by overlap with the shape or a circle's inscribed
  polygon; absence misses the possibly reached ones for the shape grown by
  `KNIFE_EDGE_METRES` (`dilate_outer` of the radius plus it). Never verify
  a reach with the circumscribed polygon.
  Tessellated scopes and obstacles are bracketed by their chord deviation
  `d` (#302): witnesses avoid the band footprint over the band widened by
  `d`, grown by `d` (`dilate_outer`); absence is proven against the sure
  core (`walkable::solid_column`, a column of height `2d` a deviation above
  the band's bottom, less the boundary's shadow within it, eroded by `d`);
  the scope is `Scene::scope` eroded and `Scene::room` grown. Never prove
  absence against the grown footprint or witness against the core. A body
  `VerticalBounds::clear_of` the band (certified by `with_extent_bounds`)
  occupies nothing. The morphology runs in a frame at the scope's corner
  (`Region::translate`): at georeferenced coordinates the overlay refuses
  its own slivers (`ZeroArea`). `Search::Undecided` with anything bracketed
  is `InexactPlacementEvidence`, never a verdict; the locator names the
  bracketed bodies (`within_deviation`).
  Placement evidence cites the scope's source. Clearance tests each obstacle's solid
  against the volume's prism shrunk by `CONTACT_TOLERANCE_M`: a triangle
  meeting it (band-clipped, then plan separating axes) or the centre inside
  (winding number) obstructs. Never go back to testing height range and
  plan outline separately: an L-shaped body satisfies both from different
  parts. Obstacles whose box reaches the prism must be closed, consistently
  and outward wound, or the request refuses.
- `src/circulation.rs` answers `map_circulation` for the free-space
  service: the placement scene's free area, pieces from `erode_inner` less
  `dilate_outer` (proof of connection), possible pieces from `erode_outer`
  less `dilate_inner` for half the width less `KNIFE_EDGE_METRES` (proof of
  separation; never drop the margin, or a gap exactly as wide as the path
  snaps shut and reads as blocked), contacts by footprints grown with the
  matching one-sided dilation, and `axiolid_route::skeleton` per piece.
  Merged scopes and a band start come from the request through
  `floor_scene`, as for placement.
  The skeleton is called directly (axiolid-triangulate 0.3.1 carries the
  fix for axiolid/kernel#190); a piece whose skeleton the kernel refuses is
  unmapped, never guessed. Half widths are distances to the free area widened by the grid
  snapping. Swept sectors are in the scene as for placement: pieces avoid
  the circumscribed polygons, possible pieces the inscribed ones, and a
  half width runs from the free area less the former to the free area
  less the latter.
- `src/containment.rs` answers `assess_containment` for the free-space
  service: the overlay difference of the footprint's bounds (the exact
  rectangle, or a cylinder's inscribed and circumscribed 64-gons) less the
  scopes' projected triangles, so the part outside is measured directly
  rather than as a small difference of two large areas, which rounding could
  push past the area epsilon for a box flush with a wall. A scope without a
  mesh or tessellated refuses.
- `src/space.rs` implements `SpaceService`: seven independent space
  measurements over storey-assigned, role-tagged geometry. A cap, boundary
  or overlap request naming its elements replaces the declared default,
  and an unmeasured requested element refuses. Unmeasured objects refuse a
  space's measurement only where they could change it (`complete_near`:
  the aspect's candidates whose host-declared bound,
  `with_unmeasured_bound`, meets the space's extent, its plan extent for
  boundary gaps, its cap plane for caps), and the refusal names them
  (`SpaceError::Unmeasured`). One without a bound may be anywhere and
  refuses every space; never place it by its storey, which bounds nothing.
  Unallocated regions still refuse for any unmeasured declared object;
  support counts read no body and never refuse. Unallocated regions are the
  polygons of each storey's floor footprint less its spaces' footprint, one
  per connected region, each relating the bodies its rings touch.
  Duplicates, overlaps and cap coverage measure plan areas as clash pairs
  do (#210, #217): `plan_area` and `shared_area` are
  `planar::bounded_footprint` and `bounded_plan_overlap`, `[lower, upper]`
  with the refused slivers' area (only slivers whose box meets the other
  footprint's). A decision is taken only where both ends agree (`contains`
  for duplicates and the containment class, `intersects` and the dust
  floor for overlaps), else `SpaceError::Refused`; a reported area may not
  move by more than `planar::snapping_area` of the space's footprint; a
  cap covered past its area within that snapping is the whole area
  (`within_rounding`). Never default a refused plan area to zero (a zero
  area read "not mutually contained" and passed duplicates), and map every
  overlay refusal to `SpaceError::Refused(overlay_refusal(..))`, never
  `Unavailable`. Spans and heights are checked before any overlay runs, so
  a pair they rule out never refuses, and so is the plan box
  (`apart_in_plan`: enclosing extents grown by the chord deviation): a
  candidate apart from the space is never measured, so a footprint the
  overlay refuses elsewhere refuses nothing here (#304). Tessellated
  spaces and candidates are bracketed by their deviation (#305): vertical
  bounds from `AxiolidGeometry::vertical_bounds` (mesh ± `d`, narrowed by
  certified extent bounds), plan areas widened by `deviation_band` (per
  outline edge `2·d·ℓ + π·d²`, clipped to the other footprint's box);
  clear height and cap coverage are intervals (`try_bracketed`),
  duplicates decided only where both ends agree, a possible overlap of a
  tessellated pair and a boundary point within a deviation of a
  tessellated outline refused (`SpaceError::InexactEvidence`). `tests/space.rs`
  pins a room with a leaning face (`leaning_room`) in all three aspects.
  `projected_polygons` winds each triangle by its area about a vertex
  (`triangle_area`), never by `ring_area`'s sum over the raw coordinates,
  whose products swamp a thin triangle's area at georeferenced
  coordinates and flip its winding (#304).
- `src/envelope_membership.rs` derives envelope membership: an object bounds
  the envelope when its plan footprint meets a bounding space. The bounding
  spaces are the request's (the rule's selection), never host-declared; a
  bounding object without a mesh refuses the derivation. Declarations are
  plain `ObjectId` data, so this stays a geometry adapter.
- `src/plan_area.rs` implements `PlanAreaService`: footprints and footprint overlaps from the plan overlay. A tessellated mesh widens the area by `2·P·d + π·d²`; never report it as a point. A declared group (`with_group`) measures the union of its members; a member without a body, an unmeasured member or undecided membership refuses, never zero. An uncovered area brackets the grown cover between an inscribed and a circumscribed 16-gon (growth by `r − d` and `r + d` for a tessellated cover), handed to the overlay as fan triangles: its ring check refuses a vertex on the line through a non-adjacent edge. Never use the 0.3.0 overlay's `Region::dilate`, whose side of the true offset is unstated.
- `src/elevation.rs` (internal) answers `measure_elevation_cover` for the plan-area service in the request's `(s, z, t)` frame: a cover body counts by its surface clipped to the subject's widened depth slab plus its cross-sections at both slab faces (the even-odd fill of the surface beyond each face, projected; never the clipped surface alone, which loses a body passing through the slab), grown exactly by the rectangle's Minkowski sum (edges swept over it); a frame covers by the convex hull of its members' projections. Tessellation and an axis off the coordinate axes (rounding) are bracketed: less growth and a narrower slab for the sure part, more and wider for the possible, the subject's band on both ends; a growth under the uncertainty drops to zero in that direction and the sure part loses its outline's band.
- `src/plan_span.rs` also answers `measure_rectangle` with the overlay's exact calipers (`minimum_area_rectangle`, axiolid-overlay ≥ 0.3.2): a tessellated footprint is `Unproven`, several least-area orientations `Tied`; the kernel's `error` widens centre and half extents and turns the axes by `asin(e / (L − e))`, except along the coordinate axes, where the extremes are recomputed exactly. That computation is `least_area_rectangle`, the crate's one rectangle: sections and the shelf layout call it too, never `minimum_area_rectangle` directly. `src/plan_area.rs` measures `measure_outside_bands` as the overlay of the subject with band hulls clipped to their shared stretch, the clip moved by `CUT_MARGIN` inwards (sure) and outwards (possible); a tessellated band member refuses.
- `src/coverage.rs` answers `measure_coverage` for the plan-area service: each effect bracketed between inner and outer regions (`Region::dilate_inner`/`dilate_outer` for grown effects; the exact visibility polygon cut to inscribed and circumscribed 64-gons; convex cells of the free region judged by the distance map with the 1-Lipschitz bound `D(g) ± ρ`, split up to `MAX_DEPTH`, undecided cells outer only; the reached cells are united by `union_soup` into a region). Effects are `Region`s, united and clipped with region operations; the trapezoids only cut the free region into convex cells. Tessellated subjects and blockers refuse; a tessellated, unmeasured or centreless source is an unmeasured effect, never an empty one. A request's connections (`connected`, `passages`) join the free region before the blockers are taken away, certain ones in the inner region, all in the outer; a bodiless passage joins through the void registered with `AxiolidPlanAreaService::with_opening_void`. A connection that cannot be measured exactly is left out and keeps the upper bound at the whole footprint, never ignored. With connections, travel cells are cut from the joined region's part inside the subject only (the distance map still spans the whole joined region), so the budget is not spent on cells the clip throws away.
- `src/sight.rs` implements `SightService` with the kernel's `line_of_sight`. Blockers apart from the box of the eye and the target are not handed over; tessellated targets in range and tessellated blockers that may matter refuse; an unmeasured blocker refuses. Never merge blockers into one mesh to decide more: occluders must stay attributable.
- `src/plan_span.rs` implements `PlanSpanService` over the plan-area service's footprints: longest diagonal and farthest span from convex-hull vertices (exact because distance is convex), centres from the overlay centroid. Tessellation widens a diagonal by `2d`, a farthest span by both deviations, a centre by `b·(R + d)/(A − b)`; a footprint no larger than its band has no bounded centre and refuses. A located centre is `Inside`/`Outside` only when farther from every footprint edge than its radius plus the chord deviation; otherwise `Undecided`. Recesses are the pockets between the outer ring and its convex hull, walked between consecutive ring vertices on the hull's boundary (vertices on a hull edge count, or a niche's mouth would be the whole wall); pockets under `RECESS_RESOLUTION` are grid rounding. Sections intersect the footprints and take their sides from `least_area_rectangle` (`Enclosing::sides`), which answers only for a unique orientation: a tied section refuses. Both refuse tessellated footprints: never bound a least-area rectangle's short side by a chord band, it is not monotone.
- `src/support_coverage.rs` answers `assess_support_coverage` for the free-space service like `containment.rs`: every support triangle facing up (positive plan orientation) is clipped to the request's elevation band and projected, and the footprint's outer bound less those tops is `Supported` when nothing is left, its inner bound `Unsupported` when something is; a support without a mesh or tessellated refuses.
- `src/side_distance.rs` (internal) answers `measure_side_distances` for the plan-span service from `measure_rectangle` and each candidate's plan triangles: each triangle is clipped, in the side's frame, to an inner rectangle (surely in the strip: sure presence, upper bound) and an outer one (possibly: lower bound), both moved by one slack covering the centre radius, the axis error over the reach, the chord deviation and rounding; the evidence is never exact. A slack above `MAXIMUM_SLACK` refuses. Never drop the slack or compare a strip edge exactly: a flush wall touches the neighbouring strips, which the request's inset, not a smaller slack, keeps out.
- `src/corridor_end.rs` (internal) answers `measure_corridor_ends` for the plan-span service with `axiolid_route::skeleton` (prune 1.5; spacing refined below an eighth of the narrowest width found, at most 50 000 samples). Collinear vertices are dropped first so a straight wall is one edge. A wall is `Decided` only when the end lies within its clearance plus two spacings of the kernel's wall ahead and rays within `SPREAD` (20°) of the path's direction all meet that wall first; never widen `SPREAD` or the slack to decide more walls without a proof. A tessellated space refuses (its edges are chords); a tessellated subject widens gap by `d` and facing by `2d`. Evidence for the ends is always approximate.
- `src/adjacent_across.rs` (internal) measures `adjacent-across` for
  `derived_relationships.rs`: a declared separating element is a flat slab
  or a straight wall (one plan rectangle) or refuses; spaces are measured
  through `walkable::band_footprint` against a strip along each wall face
  (overlap along the wall) or a band above or below a slab (overlap area
  with its footprint), each twice, shrunk and grown by rounding plus both
  chord deviations. Decide a face only when both brackets agree; never
  drop the margin or decide from one bracket.
- `src/derived_relationships.rs` implements `DerivedRelationshipService`:
  element to containing (or nearest) space, opening to the spaces a probe
  first enters on each side, space to larger covering spaces, space to the
  levels it spans. Spaces, openings and levels (with their height bands) are
  host-declared `ObjectId`s; bodiless openings are probed through a
  host-supplied void. Undecided cases (unmeasured or bodiless spaces, points
  on a boundary, ties, tessellations within reach, an extent straddling a
  level's needed reach, an unmeasured level) refuse the whole answer.
  `spans-level` reads only vertical extents, so it widens a tessellated
  space by its chord deviation instead of refusing it; never decide a level
  from the mesh's own extent alone. `intersects` is symmetric and pairwise
  (`Walk::step`), over the request's universe with no declarations: a
  pair is decided once from `AxiolidProximityService::measure_proximity`
  over the same geometry (built lazily), never from extents or the mesh
  alone. Sharing volume needs containment, a witness deeper than the
  combined chord deviation or a certified shared volume above
  `TOUCHING_VOLUME`; touching bodies share nothing, and anything else
  refuses.
- `src/vertical_extent.rs` implements `VerticalExtentService`: bottom and top elevations of a mesh's used positions. A tessellation widens each by its chord deviation and is never exact, even at zero deviation. Directional extents project the same positions; only a coordinate axis projects exactly, any other direction widens by the dot product's rounding bound and is approximate. Face pieces (`face_normals::measure_pieces`) group the face's triangles by shared edges (exact coordinates) whose normals agree within `COPLANAR` plus both triangles' leans, so a stated crease always parts two pieces; the area widens like `facade_area`'s for a tessellation, and only a planar mesh gives exact pieces. The whole boundary is refused for an open mesh.
- `src/walking_surface.rs` implements `WalkingSurfaceService`: ramp runs are connected planar upward faces flatter than 45° of an exact mesh; tessellated ramps, headroom subjects, landing ends and nearby tessellated obstacles refuse, and headroom is never exact. Sides and landing extents come only from `rectangle`: boundary edges (matched by exact coordinates) on the spanned rectangle's sides and the plan area equal to it, within a `slack` only for a tessellated tread (twice its chord deviation, and every side widened by it); never report a bounding box as a width. A landing is one connected level surface of one candidate (or a ramp's own, never a flight's treads) meeting the end's strip within `LANDING_REACH`; several are refused, never merged. A turning flight's end is placed in its end tread's frame (`flight::TreadFrame`, from `framed_flight`), square to its nosing, never along arc lengths; an end on a winder refuses. Its handrails are measured part by part (`handrail.rs`): a rail must fill a rectangle along exactly one part, on one side, or the request refuses; its pitch line is the nosings' ends on that side lying on the part's side line, and beyond them, around a turn, the rail is measured against both neighbouring elevations (never interpolated, never past the flight). Parts are runs of treads sharing one frame; flight.rs gives a tread the frame below when their nosings are parallel within their uncertainty, so never compare frames by tolerance elsewhere. The clearance below leaves out the plan regions where the subject lies flush on the space's floor (convex `subtract`), never the whole face pair. Handrails (`src/walking_surface/handrail.rs`) must fill one rectangle along the walking direction with their upward faces, or the request refuses; heights come from the upper envelope of the rail's edge lines between breakpoints (ends and pairwise crossings), never from sampling. Tessellated rails are measured and widened by their chord deviation; lines that do not rise are evaluated without rounding, so a level extension rises by exactly zero. Warped runs refuse. Clear widths (`src/walking_surface/clear_width.rs`) clip obstacle triangles to the band's convex pieces (one per pitch-line segment) and evaluate the free width only at clipped vertices and the ends, never by sampling; the lower bound uses the grown band with every obstacle grown by the largest chord deviation, the upper bound the shrunk band without tessellated obstacles. Keep both: dropping either direction's growth stops the interval holding the true width. An obstacle over the middle or possibly enclosing the band refuses; a turning flight refuses. A landing's clear width (`landing_clear_width`) runs the same `narrowest_over` on a level pitch line from the landing's arrival line to its far side, between its rectangle's sides, from `measure_landing`; a landing filling no rectangle refuses, and each side's `bounds` are the obstacles with a section in the grown band on that side, so a wall flush with the slab's edge bounds it and one standing off it does not.
- `src/flight.rs` (internal) measures flights of a closed, outward, one-piece mesh, exact or tessellated, from `axiolid_inspect::detect_planes`: treads are upward planes whose certified deviation and corner spread stay within `LEVEL_TOLERANCE` of the coordinates plus twice the chord deviation, and every position of a tessellated flight widens by the chord deviation. Walking directions and lines come from the treads, never the placement. A turning flight's positions are floating-point constructions widened by `MARGIN` over the crossing's sine, so never exact; grazing crossings refuse. A tread's sides come from `rectangle` along its walking direction (the flight's, or square to its nosing on a turning flight); a winder tapers and fills none, so it has no sides and its flight no width. Riser closure: `Open` needs a face falling away from the lower tread's back line, `Closed` a rising face along all of it; the first riser is `Closed` only when coplanar faces cover the strip below the nosing to the base, otherwise `NotMeasured`, never `Open` (a set-back riser looks the same). Test fixtures build closed meshes with `stepped` (height field over plan cells) and `voxels` in `tests/walking_surface.rs`.
- `src/boundary_coverage.rs` implements `BoundaryCoverageService` over
  host-registered boundary surfaces keyed by space and boundary. Body faces
  group by oriented plane; a boundary triangle counts on the nearest face
  plane its corners all lie within the tolerance of, and a boundary with a
  triangle on none is `OffSurface`, reported and covering nothing. Per plane,
  faces and boundary parts are overlay `Region`s (union, intersection,
  difference; overlay 0.3.3 accepts its own outputs): uncovered is the faces
  less the boundaries' union, measured directly, covered the rest of the
  faces, overlap accumulated boundary by boundary. Only an axis plane all
  face points share projects exactly; others widen by rounding plus the
  overlay's grid snapping (`OVERLAY_SNAP`, #173), tessellated boundaries by
  their deviation, both as `2·P·w + π·w²` bands. Never drop those margins:
  the turned-box test fails without them. Curved, unmeasured or bodiless
  spaces and unmeasured boundaries refuse.
- `src/triangle_count.rs` implements `TriangleCountService`: the triangles of the registered mesh. Exact evidence only for a planar mesh; bodiless counts zero, unmeasured refuses.
- `src/facade_area.rs` implements `FacadeAreaService`: steep faces that look outside, classified at four samples each. A face held against another body or a host-declared space, or whose ray first meets itself or a space within `REACH`, is not facade; meeting nothing or another body is. A face whose samples disagree widens the interval, never a guess. Unmeasured bodies anywhere, bodiless declared spaces and tessellations within reach refuse. `measure_face_area` sums the triangles of each plane (parallel normals either way wound, corners within `ON_SURFACE`) and answers the largest, exactly, for exact meshes only: a tessellated body's planes are chords, so it refuses; never widen a chord plane into a face.
- `src/walkable.rs` (internal) builds walkable plan domains for the two route
  services: floors (closed exact bodies with one horizontal underside), band
  footprints (what a body occupies inside an open headroom band: the boundary
  clipped to the band plus the winding section just above its bottom), portal
  frames, sides and corridors, mid-line chord bounds, and sweep proofs.
  **Never use `Region::erode` (0.3.0)**: it does not state its side. The
  module's own `erode` removes an enclosure of the boundary's disc sweep and
  only proposes paths; `sweep_inside` proves them with an outer enclosure
  grown by `MARGIN`, which must stay above the overlay's grid snapping.
  Width upper bounds come only from chords of a portal's mid-line.
  Plans are `Plan`: overlay output, handed back to the overlay as it is
  (settled output is a valid operand from axiolid-overlay 0.3.3) and
  validated and settled with a tight tolerance, since thin slivers fail at
  `ON_SURFACE`. Band footprints hand the overlay triangle fans of the
  clipped faces (`projected_fan`, collinear ones left out), never the
  clipped polygons: a face on edge projects to a ring the overlay refuses
  as self-intersecting (#222). They are united in one overlay (crossings
  the subject, the section the clip) at `snapping_tolerances` (a few ulps
  of the coordinates, coarser up to `ON_SURFACE`), else in two steps,
  because the overlay fails to link arrangements with corners a few ulps
  apart; a refusal names the obstacle. At coordinates where eight ulps
  exceed `ON_SURFACE` (georeferenced, #304) the ladder runs from there
  up `SNAP_STEP²`, never past `SNAP_CEILING` (`MARGIN / 100`). `trapezoids` remains for convex pieces (an inner point,
  placement obstacles as rings). Test against the published crates, not a
  local kernel checkout.
- `src/walkability.rs` implements `WalkabilityService`: a region per surface
  (a hub) and per portal face (a landing). Every definite passage must end
  where the next begins, or a definite route could transit a surface without
  a proven sweep; never add a hub-like region whose passages are definite
  without a sweep from a shared point. A door's leaf and lining are unknown,
  so a crossing is definite only for a bodiless opening or a stated clear
  width (`with_clear_width`, or the request's
  `with_stated_clear_widths`; the narrower of the two counts, and a stated
  width bounds a void's crossing from above). A connector passage is
  definite only as `climbs` measures it: a stair or ramp from
  `connector.rs` between the surfaces its landings stand on, with its width
  admitting the body, headroom (`connector::Climb::passable`) against the
  request's obstacles clearing the band, and sweeps hub to lower landing and
  upper landing to hub proven; a lift (`rides`, exact bodies only) between
  two surfaces when the body at the first hub lies in the lift's plan and a
  sweep from there to the second hub is proven. Every other connector pair
  stays possible without a lower bound; never make one definite without a
  sweep from a hub. Swept sectors (`Ground`): hubs stand clear of every
  circumscribed swing, a portal's landings, spokes and crossing clear of
  every swing but its own (`Ground::past`); mid-line bounds ignore swings.
  A surface touching no other and joined by no connector is split into
  possible pieces (`Parts::split`: `erode_outer` by the radius less
  `MARGIN`, less obstacles and inscribed swings `dilate_inner`ed by as
  much); a face reaches the pieces within the radius plus `MARGIN` of its
  half band and its own swing (`dilate_outer`), and the hub's piece always,
  with width zero when out of reach, and faces whose zones meet are joined.
  Never split a surface that touches another or a connector joins: a body
  may stand across the shared boundary.
  Pieces in one footprint polygon are joined by a stretch passage (upper
  bound the width less `2·MARGIN`, a `WalkableStretch` located in the
  footprint left uncovered by both pieces' `dilate_outer` by the radius,
  else at their closest points); its limit comes from re-splitting without
  overhead obstacles (`Low`), then without any obstacle or swing
  (`Obstructed`), else `Narrow`. Such a pair's hub gets no zero-width
  spoke; a face reaching no piece keeps it, with a stretch in front of the
  face. The limit and relations only word the block; the separation is the
  proof. The obstruction depth tolerates obstacles only against the
  floor's footprint, bracketed by `erode_outer` (`Ground::plain`, witnesses)
  and `erode_inner` (`Ground::wide` and `sure`: mid-line bounds and
  pieces); never swap the two. Surfaces within the surface gap are joined
  like touching ones (and never split), the witness crossing the gap
  through the intersection of both footprints' `dilate_inner`, less
  `Ground::raw` (untolerated obstacles over the floor grown by the gap)
  and circumscribed swings.
- `src/metric_routing.rs` implements `MetricRoutingService` on the origin's
  level. `Blocked` needs complete evidence (every declared surface and portal
  measured, no connector touching the level) and a separation of the free
  region less the chord-proven bands around narrow portals' mid-lines. The
  `axiolid-route` path is a lower bound only: it can cut across outside gaps
  between collinear boundary edges. `nearest_target` and `farthest_point`
  build one `distance_map` over the level, which then also holds the
  corridors of portals opening from it onto nothing else of it (exits to
  the outside); a target may stand in a portal. A target the service cannot
  place counts only by its straight line; a lower bound comes from the map
  only on a closed level with every target placed, and so does an
  unreachable verdict. Farthest points are for a point body only; never
  report the bracket for a body with a radius. Avoided objects
  (`avoids_objects` is true) join the obstacles through `obstacles` and
  the band cut, surfaces and portals included; never drop an avoided body
  that cannot be measured, refuse. `trace_path` measures each object's plan
  footprint (a surface's floor, a body's projected triangles, a
  tessellation's grown plan box as an upper bound only) with
  `walkable::segment_cover`: the upper bound keeps pieces within `MARGIN` of
  the boundary, the lower bound drops them. Keep the upper bound never
  short of the length along the boundary.
  `src/metric_routing/weighted.rs` answers weighted travel (on one level
  and, through `climb.rs`, across levels) and forced walks on one level
  (`axiolid-route` 0.3.5, axiolid/kernel#195, #196, #198): costed
  objects (`Costed`, measured once per request) have exact footprints cut
  to a level's free region and handed over as cut (the kernel takes a cut
  edge meeting a wall up to rounding as touching it; never snap vertices
  here), spacing from the tolerance within `COST_POINTS`. A cost weighs
  only the levels holding a floor in whose storey (`storeys`: from the
  highest floor top below, or `REACH` under the floor, to its top, both
  open) its body lies; never apply a cost to every level, which would
  raise the lower bound of a walk on another floor. An object in no storey
  whose footprint meets a level's free region refuses there. A climb's
  upper bound counts the largest factor of a cost meeting its connector
  (`climb_factor`, boxes meeting), its lower bound factor one; never
  weight a climb's lower bound. A tessellated or bodiless costed object
  or a radius refuses, and a weighted request must never be answered with
  a plain map (a level no cost lies on is plain by definition). Forced
  walks need a closed level with every target placed; their upper bound
  only a point and an exact footprint.
  A request with a `ConnectorRouting` goes to `src/metric_routing/climb.rs`:
  only its own connectors are climbed (host `with_connector`s are ignored),
  each measured by `src/connector.rs`; the levels reached through them are
  joined in a small graph of level walks (one `distance_map` per landing and
  one per level's targets) and climbs, searched on lower and on upper
  bounds. A level walk's lower bound is infinite only where `separated`
  proves it on a closed level; an unclimbed requested connector opens every
  level it touches; a connector whose passability is undecided keeps its
  lower bound and loses its upper. Farthest points weigh each landing by
  the walk beyond it and bracket with weighted-target maps
  (`distance_map_within_weighted`, axiolid/kernel#197): one seeded with the
  walks beyond bounded from below for the lower bound and the unreachable
  verdict, one seeded with those bounded from above (a source without one
  left out) for the upper bound, one map when both agree; per-source maps
  only decide which sources a map holds and bound from above when no
  weighted map can be built. Never seed a lower-bound map with upper
  weights, and never report a witness's value without taking the least over
  every source, straight lines for a source the map cannot show. With
  costs, every one of these maps is built by `seeded_map` over the stage's
  own cost regions (`Stage::regions`), weighted maps seeded
  (`weighted_distance_map_seeded`) where the plain ones were; a weighted
  map that fails for anything but a target outside refuses, never falls
  back to a plain map.
- `src/connector.rs` (internal) measures stairs and ramps for walks between
  levels: an exact straight flight whose first and last treads have sides,
  or an exact ramp of one run with sides; the walking line runs between the
  ends' midpoints, landings stand `LANDING_GAP` plus the body's radius
  outside them. `passable` refuses only a flight surely narrower than the
  body or headroom surely below its height, and leaves anything unsure
  undecided. Lifts are never measured here. It reads flights, runs and
  headroom through `walking_surface`'s `tread_flight`, `sloped_runs` and
  `headroom`, the same measurements the walking-surface service answers.
- `src/shell.rs` (internal) reads closure from positions, not indices
  (#308, #309): corners with equal coordinates (`-0.0` as `0.0`) are one
  vertex, never corners within a tolerance. `welded_where_closed` is
  applied by `with_mesh` and `with_tessellated_mesh`: it shares repeated
  corners only when that makes a mesh a closed two-manifold it was not,
  keeping positions and triangles. It never welds a mesh closed by its own
  indices, nor keeps a weld that joins shells along an edge or at a corner
  (`single_fans`): touching items would become one non-manifold shell and
  could no longer be measured shell by shell (#312). `pieces` splits a mesh into the pieces
  its surface connects; a piece is closed when its triangles, zero-area
  ones counted and those repeating a corner left out, use every edge as
  often one way as the other, so its winding number is whole off the
  surface. `walkable::band_footprint` needs that only of the pieces
  reaching from below the band into it, `solid_column` of those standing
  across its level; other pieces count by their surface. Never claim a
  piece closed from a weld within a tolerance, and never take the section
  of an open piece.
- `src/planar.rs` (internal) holds the plan-projection helpers shared by the
  services; `src/geometry.rs` holds the mesh store and triangle vocabulary.
- `src/linear_quantity.rs` implements `LinearQuantityService`: parallel shelf
  bands on the footprint less each requested door's clearance (its footprint
  grown by clearance plus plan gap, circumscribed for the lower bound, inscribed
  for the upper), along the footprint's and every door's least-area
  rectangle (`least_area_rectangle`; any orientation, tied or unproven, only
  proposes a layout), anchored at either wall; the longest layout counts. Doors are the
  request's (the rule's selection), never host-declared; a bodiless opening
  needs its void. Bands are measured exactly between breakpoints; keep the
  lower bound's grown cross-sections and shrunk runs, and the upper bound's
  reverse, or the interval stops holding the true length. Keep the 1 mm
  `CONSTRUCTION_TOLERANCE` above twice the margin: without it a band against a
  wall starts on the boundary and the lower bound loses every wall band.
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
- `src/proximity.rs` implements `ProximityService` for clash and distance
  checks: separation from `closest_points_on_triangles`, plan overlap from
  the oriented footprint overlay in `planar.rs` (only when the request asks,
  `ProximityRequest::with_plan_overlap`: an overlay per pair), and penetration witnessed by
  sampling points (including midpoints between an edge's crossings of the
  other surface, via `axiolid-ray-mesh`) against winding numbers. Only closed
  two-manifold meshes have an inside; a pair with one is measured, two open
  surfaces report no penetration. A witness must be a point of the body it
  reaches from: surface samples always are, but the interior candidates
  (vertex centroid, chord midpoints through the largest triangle) are taken
  only when the body's own winding number places them inside, clear of its
  surface. Never add a non-surface sample without that check: a body with a
  hole has its centroid in the hole (#315).
  Fidelity comes from `AxiolidGeometry::with_tessellated_mesh`. A
  host-declared bodiless object is refused with `ProximityError::NoBody`,
  never `Unavailable`, so a comparison can tell "no body" from "unmeasured".
  An overlay refusing the footprints leaves `measure_proximity`'s plan
  overlap `None`, never the pair unmeasured. The footprint relation
  (`relation`) measures `planar::bounded_plan_overlap`: shadows the overlay
  would refuse as slivers (corners within its tolerance, area within its
  square) are left out and their area, rounding included, bounds what
  they could add; an exact pair they could tip is refused with that
  reason. Every remaining overlay refusal is `ProximityError::Refused`
  with the overlay's error (`overlay_refusal`), never `Unavailable`.
  Never drop a shadow with area without counting it into that bound.
  `body_surface` hands out the registered mesh unchanged (world
  coordinates, its fidelity), and `measure_surface_distance` measures the
  service's own body against a surface from any session with the kernel's
  certified `hausdorff_distance` (axiolid-measure 0.3.5, both one-sided
  intervals with their witnesses). A tessellated subject or counterpart
  refuses (`EvidenceFidelityMismatch`): a chord deviation bounds the true
  surface from the mesh one way only, so never widen it into a surface
  distance. `tests/surface_distance.rs` measures two sessions' walls.
  `body_surface` also hands out the object's registered exact boundary
  (`ExactBoundaryHandle` over the shared `Arc<ExactBRep>`), and where the
  subject has one and the counterpart's downcasts to `ExactBRep`,
  `measure_surface_distance` measures between the boundaries first (each direction with
  `one_sided_boundary_hausdorff_with_budget`, axiolid-measure 0.3.7, or
  `one_sided_body_boundary_hausdorff_with_budget` for several items, 0.3.8,
  whose layout refusals fall back to the meshes; a perturbed body never
  takes this path, an exact boolean does, widened by its rounding,
  `Tolerance::METRE`, at most `BOUNDARY_HAUSDORFF_SPLITS` splits a side,
  each widened by `certified_directed`'s rounding bound,
  `try_from_boundaries`), whatever the meshes' fidelity; a kernel refusal
  or ill-formed interval falls back to the mesh path, never to a guess.
  Identical and translated copies close without a split, prisms from
  `boolean_arc_prisms_exact` included (axiolid/kernel#227); a turned or
  reshaped copy closes only at first order and stops at the cap with a
  sound, wider interval (a square column turned 0.01 rad: about 2 mm to
  12.6 mm, seconds in a debug build). Never narrow the interval here and
  never raise the cap to the kernel's own 200,000: the comparison judges a
  wide interval by the straddle rule. `tests/boundary_surface_distance.rs`
  measures two sessions' round columns.
  A tessellated pair whose objects both have an exact boundary
  (`AxiolidGeometry::with_exact_boundary`/`with_exact_body`) also gets the
  kernel's certified `boundary_distance` (`body_boundary_distance` over
  several items or a placed body; axiolid-measure 0.3.4/0.3.8, `exact`
  feature), widened by both bodies' perturbation and rounding, to
  `CERTIFIED_ACCURACY_METRES`, widened by a rounding bound on its witness
  points, and reports the intersection with the chord-widened interval
  (`with_certified_separation` in `measure_proximity`, the interval itself
  for `Minimum3d` in `measure_distance`); an empty intersection refuses
  (boundary and mesh disagree), a kernel refusal keeps the chord interval.
  The same pair's plan relations (`proximity::Boundaries`) are certified
  by the kernel's plan measurements under the same rules (a perturbed pair
  never by `plan_overlap`), the one-solid queries for bodies of one solid
  in the world and the `body_plan_*` ones (axiolid-measure 0.3.9,
  axiolid/kernel#237) over every item pair otherwise: `Horizontal` by
  `plan_boundary_distance`; the footprint relation of `PlanOverlap` and
  zero-offset `Vertical` (`relation`) by `plan_overlap`, asked in both
  orders because its search is order-dependent (`Undecided` keeps the
  mesh's relation, a contradiction of a decided one refuses, a gap must
  exceed rounding and the bodies' rounding); a positive offset by
  `plan_boundary_clearance`, intersected with the chord-widened plan
  distance. Never report a certified interval alone, never certify an
  exact pair (its evidence is a point), and never use the distance in space
  for a plan projection or the plan distance for one in space.
  `measure_region_distance` folds the same 2D closest points between a
  stated region's fan triangles and the counterpart's footprint, widened by
  the counterpart's chord deviation.
  `measure_distance` answers projections: horizontal distance folds 2D
  closest points over projected triangles (edge-on ones as segments), exact
  for non-convex footprints; vertical is the extent gap of footprint-related
  bodies (one-sided with a direction, whose side a tessellated end within the
  combined deviation leaves open); plan overlap uses the overlay. `src/vertical_surface.rs` (internal) measures `VerticalSurfaces::Between`: levels from the mesh bounds (widened by the combined deviation, the side open within it), and `Nearest` from counterpart triangles clipped in plan to each projected subject triangle, pieces of positive plan area only (a body touching the footprint's edge stands over none of it), the minimum at a piece's vertex; a tessellation takes its lower bound from the footprint grown by the deviation and its upper bound only from a witness centroid deeper than the deviation inside the footprint, never from a boundary vertex. Overlap extents span witnessed
  intersection points (edge crossings both ways, inside vertices tried
  outermost first) below and the boxes' overlap above; `measure_overlap_along` spans the same witnesses projected onto each stated direction below and the overlap of the bodies' own ranges along it above, widening every projection off a coordinate axis by its rounding bound (`along`), never reported tighter; the Hausdorff
  distance is bounded per triangle by the farthest vertex from one other
  triangle, split twice at most. Never report either as a point you did
  not bound. Intersection volumes come only from `axiolid-inspect`'s
  certified integrals (`intersection_volume`, `enclosed_volume`), for two
  closed solids; a kernel refusal leaves the volume `None`, never zero, and
  a tessellation widens it by `tube_volume`, never by nothing.
  `measure_body_volume` is one closed body's `enclosed_volume` widened
  the same way; an open surface refuses. A pair the whole-mesh volume
  refuses where one body is several closed shells (index-connected, each
  closed and measured, `proximity/shells.rs`, split once per object and
  cached) and the other one closed body the kernel measures, both exact,
  is bounded shell by shell (#312): own volume from the largest shell to
  the sum, shared from the largest share to the sum, and the volume
  outside the other body from the largest shell's outside part to the sum
  of them, attached with `with_subject_outside`/`with_counterpart_outside`.
  Never sum shells into the shared volume's lower bound (they may overlap),
  and never read a tessellated pair this way without widening every shell. A tessellated overlap is asserted
  only from a witness deeper than the deviations and denied only beyond the
  combined deviation; otherwise it stays open. Never decide it on the mesh.
  Zero-area triangles (#221) go through `src/proximity/zero_area.rs`:
  every distance to one (`point_triangle_distance`,
  `triangle_pair_distance`) is the distance to its edges and every edge
  crossing (`segment_hit`) meets it only within `LINEAR_TOLERANCE` of an
  edge; one whose corner lies farther than that off its longest edge is
  refused by name. Never call the kernel's triangle or ray primitives on a
  body's soup directly, and never skip such a triangle in a distance: it
  stands for its segment. A body is `solid` when its triangles, zero-area
  ones counted and those repeating an index left out, use every edge
  twice, once each way (`closed_chain`); its winding number
  (`Body::winding`) is taken without the audit's degenerate triangles, and
  a point within reach of the area left out (shift `A / (4π d²)` of a
  quarter or more) witnesses nothing and refuses a containment by name.
  Kernel refusals of the measurement are `ProximityError::Refused` with
  the kernel's reason (`volume_refusal`, `ray_refusal`); keep
  `Unavailable` for an object without a readable mesh.
- `src/face_distance.rs` answers `measure_face_distance` for the proximity
  service. Face classes come from outward normals (outward by the host's
  signed volume); a triangle within `CLASS_TOLERANCE` of 45° refuses the
  classes it might belong to. Only a body proven wholly inside (every vertex
  inside, surface clear of the host's) gets the exact triangle distance;
  otherwise the interval's lower bound spans the outside part (vertices not
  proven inside, surface crossings, nearby host vertices) face by face. The
  host must be an exact closed solid; a tessellated body widens both bounds
  and decides a vertex's side only beyond its deviation.
- **Every service honours fidelity.** Proximity reports approximate evidence.
  The services whose contracts only accept exact evidence -- contact,
  envelope, clearance and free area, guard -- refuse with their inexact-evidence
  error when a tessellation could change the answer (placement,
  circulation and space bracket it instead, above): the subject itself, or a
  part whose enclosing extent (mesh box grown by its chord deviation) comes
  within the measurement's reach (`AxiolidGeometry::tessellated_near`). A
  curved part elsewhere in the model blocks nothing. Shelf length is an
  interval, so a curved space widens it by its chord deviation instead.
  `tests/fidelity.rs` pins each case and fails without the guards.
- Proximity queries go through an `axiolid-spatial` BVH per body; skips are
  exact (box gap never exceeds triangle gap), and the unit tests compare the
  indexed results with an exhaustive scan. Penetration ranks samples by
  indexed surface distance and runs the O(n) winding test deepest first,
  stopping at the first inside point. Keep that order: it is what makes a
  19k-triangle pair take 0.26 s instead of 91 s.
- `projected_polygons` winds every projected triangle counter-clockwise. A
  closed solid's top and bottom faces project with opposite windings and,
  under the non-zero fill every service uses, cancel to no footprint at all.
  Test fixtures with same-winding caps hid this; use closed, outward-oriented
  boxes when testing plan measurements. It also leaves out every shadow
  without area (`planar::collinear`: all vertices on one line up to the
  rounding of their coordinates, such as a vertical face's), and so do the
  space-boundary projection and coverage's travel cells, before any
  `union_soup`, `Region` or overlay union. Overlay 0.3.5 to 0.3.8 emptied
  whole unions over exactly collinear rings (axiolid/kernel#219, fixed in
  0.3.9), and rounded ones are rarely exact; never hand a shadow without
  area to a union.
- **Bodiless and unmeasured are different facts.** `with_no_body` says the
  object occupies no volume; `with_unmeasured` says it has a body the host
  could not mesh. Never let a service skip an unmeasured object as if it were
  bodiless: its extent is unknown, so a measurement it could affect refuses.
  Contact measures only the request's candidates and refuses while one of
  them is unmeasured or undescribed; space refuses while a declared role,
  storey member or requested element that could reach the space is; free
  space refuses it as an obstacle. A host-declared bound
  (`with_unmeasured_bound`) only says where the body cannot be; an invalid
  one is no bound. `parts_bound` encloses the parts of a whole that could
  not be composed (mesh boxes grown by deviation, declared bounds), and is
  `None` when any part is unbounded: never skip an unbounded part.
- A geometry set may hold several sources. Evidence about one object (contact,
  proximity, facade area, vertical and directional extent, triangle count, shelf
  length, clear height, derived relationships, a walkability passage, a
  reachable metric route's origin, a tread flight, sloped runs and the headroom
  above a subject) cites that object's source; only
  set-level measurements (a walkability snapshot, a blocked route's
  completeness) take the source given to their constructor.
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

Depend only on what the registry publishes. The workspace pins `axiolid-*`
0.3.0, except `axiolid-measure` 0.3.9 with the `exact` feature (certified
`boundary_distance` and the `body_*` measurements over several items, in
space and in plan, and the plan measurements `plan_boundary_distance`,
`plan_boundary_clearance` and `plan_overlap`, with `axiolid-brep` 0.3.3 for
the `ExactBRep` hosts register, built by `exact_boundary` with
`axiolid-construct` 0.3.14, `axiolid-curve`, `axiolid-model`,
`axiolid-profile`, `axiolid-surface` and `axiolid-mesh-compile` 0.3.13's
`ReferenceExactCompiler` with its boolean reports and rounding floors
(axiolid-brep-boolean 0.1.5); the tests compile meshes with
`axiolid-mesh-compile` and `axiolid-mesh-boolean-boolmesh`), `axiolid-overlay` 0.3.10 (`minimum_area_rectangle`, the
Minkowski and dilation family, settled `union_soup` output, fast on mesh
soups, features within the caller's tolerance snapped before the exact
arrangement), `axiolid-route`
0.3.7 (`distance_map`, `farthest_point`, weighted maps, `forced_walk`, and `skeleton` behind circulation
maps and corridor ends, with `axiolid-triangulate` 0.3.1) and
`axiolid-inspect` 0.3.2 (volumes, `line_of_sight`, `detect_planes`). Check the
registry source, not the kernel checkout, before relying on an API.

## Waiting on upstream

- axiolid/kernel#173: `overlay`/`Region` snap output to an integer grid, so
  plan areas here are off by ~1.5e-8 of the extent while reported exact.
- `plan_overlap` shows overlap only from planar faces that are not
  vertical, and its search depends on argument order, so both orders are
  asked. Since axiolid-measure 0.3.9 it finds a level face over part of
  another's shadow (a sliver over a slab's edge); overlap between curved
  faces alone is still never shown.
- One-sided disc morphology (`Region::erode_inner` and friends, #163) is
  published in axiolid-overlay 0.3.2. Walkability splits surfaces touching
  nothing into possible pieces with it; surfaces touching others stay
  whole, and metric routing still refuses routes it could report blocked.
- The single-route service still treats the kernel path as a lower bound or
  proposal only; the many-target queries of `axiolid-route` 0.3.2 use a point
  path as a witness only.
- `src/circulation.rs` builds on `axiolid_route::skeleton` (axiolid/kernel#139,
  axiolid-route 0.3.3) and needs axiolid-triangulate 0.3.1 with the fix for
  axiolid/kernel#190; it calls the skeleton directly, without a worker,
  timeout or retry spacings.
- Exact booleans (axiolid/kernel#228, #234, #236): unions, intersections
  other than a half-space clip, and operands that are no placed extrusions
  keep the mesh alone; so do the general boolean's refusals. Round holes
  touching a planar face, an I-beam's root fillets included, are built
  since axiolid-mesh-compile 0.3.13 (#243, #249); a hole a fraction of the
  tolerance off a filleted flange is still refused by name when it needs
  a reading within tolerance (under axis matrices the no-tolerance run
  decides it exactly).
- Since axiolid-brep-boolean 0.1.5 (#251) a report at `Tolerance::ZERO` is
  empty or the boolean is refused (`ToleranceExceeded`). The adapter still
  treats any non-empty report as perturbed, never inferring exactness from
  a magnitude: a reading within tolerance may report a zero magnitude.
- I sections of decimal (IPE, HEA) sizes build at `Tolerance::ZERO` since
  axiolid-construct 0.3.14 (#250). The ZERO-then-METRE fallback in
  `boolean` stays for contacts that hold only up to rounding (above).
- Swept disks (axiolid/kernel#245, #248): sharp corners without a fillet
  radius are mitred again with a `Proven` bound (mesh-compile 0.3.12,
  construct 0.3.13), so such pipes are measured; still refused by name: a
  corner beside an arc, a closed polyline, a fillet radius equal to the
  disk radius (a horn torus), a mitre reaching past its leg. The exact
  compiler (and so `exact_boundary`) still refuses directrices with
  corners.
- Sweeps along an `IfcGradientCurve` (axiolid/kernel#252) are built and
  certified by mesh-compile 0.3.14; a grade break inside the sweep is
  unbounded by name. `exact_boundary` builds one only where
  `exact_directrix` reads a segment or an arc.
- Warped authored faces (axiolid/kernel#254, #257, #261, engine#213):
  since mesh-compile 0.3.14 the compiler reports polygon mesh faces and
  B-rep faces without a surface by the slab width of their corners about
  the fit plane, which hosts declare as returned. A boolean with such an
  operand is still unbounded (the exact compiler refuses polygon meshes
  and B-reps), so the host refuses a body with a warped face under a
  boolean.
- Faces are triangulated by the certified clipper (#260) under
  `PinchPolicy::Accept` (#262). A face ring touching another ring inside
  an edge a neighbouring face shares leaves a T-junction in that face; the
  mesh audit reads the edge as open, so such a body is a surface here,
  never a closed solid (`tests/proximity.rs`). Asked upstream: insert the
  touching vertex into the neighbour's edge too, so the shell closes. The
  same shell closed by a zero-area triangle along the edge is a closed
  solid to proximity (#221,
  `a_closed_box_with_a_zero_area_triangle_is_measured_as_the_box`).
- Pinched rings (axiolid/kernel#253, #262): axiolid-construct 0.3.15's
  plain `triangulate` and the extrusions refuse rings touching at a single
  point; `profile::triangulate_with(rings, PinchPolicy::Accept)` takes
  them for regions and surfaces. Nothing here hands overlay output to the
  construct triangulator: `space.rs`, `free_space.rs`, `plan_span.rs`,
  `envelope_membership.rs`, `linear_quantity.rs`, `elevation.rs`,
  `walkable.rs`, `circulation.rs` and every `Region::new` work through
  axiolid-overlay 0.3.10 (which accepts touching rings), axiolid-measure
  and axiolid-route with axiolid-triangulate, none of which depends on
  axiolid-construct (`cargo tree -i axiolid-construct`), and
  `exact_boundary.rs` extrudes only lowered profiles, which are solids and
  rightly refuse. Any future site that triangulates a region
  through `axiolid-construct` must use `PinchPolicy::Accept`.
  `tests/space.rs` (two rooms meeting at a corner, two regions of 25 m²)
  and `tests/free_space.rs` (an L round a column, a corridor narrowed to a
  point) pin it. Should a later overlay refuse touching rings too, check
  each site above against that release.
- Bodies of several items (axiolid/kernel#229): items under openings are
  cut part by part by the host's lowering (in the host's frame since
  `ifc-geometry` 0.13, openbimrs/ifc#388), so a turned
  wall of several parts touches on planes no axis is normal to and keeps
  the mesh for the surface distance.
- Certified tessellation (axiolid/kernel#231, #232, #235, mesh-compile
  0.3.10): hosts declare each mesh with `compile_mesh_with_deviation`'s
  bound, a boolean's measured against the exact compiler's result. Tapered
  extrusions, sectioned spines, composites holding other curves and the
  booleans the exact compiler refuses stay unbounded, so such a curved
  body is unmeasured at the host.
- axiolid/kernel discussion #175: winding numbers are O(n) per query; the
  deepest-first ordering in `proximity.rs` hides it in practice but not in
  the worst case.
