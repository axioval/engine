# Not-evaluated inventory

A geometric rule that cannot decide reports *not evaluated*, never a pass.
Each refusal is sound, but on real models they add up, and the rule
coverage overstates what a check judges. The inventory counts the
not-evaluated outcomes of a broad geometric rule set over a corpus of real
models, groups them by cause, and ranks the causes. The ranking decides
which case to fix first.

## Running it

The rule set lives beside the script in `scripts/inventory/`. It runs ten
geometric capabilities on every model:

| Rule | Capability | Subjects |
|---|---|---|
| `element-clash` | `clash` | building elements against building elements |
| `furnishing-in-space` | `containment` | furnishing in spaces |
| `column-wall-distance` | `distance` | columns to walls |
| `element-triangle-count` | `triangle-count` | every element |
| `wall-thickness` | `body-extent` | walls |
| `space-validation` | `space-validation` | spaces |
| `storey-space-area` | `plan-area` | storeys and the spaces spanning them |
| `space-boundary-coverage` | `space-boundary-coverage` | spaces |
| `space-free-floor` | `free-floor-rectangle` | spaces |
| `stair-risers` | `stair-geometry` | stair flights |

Its object types are bound to IFC2X3, IFC4 and IFC4X3, so every supported
release is checked. Its bounds are deliberately loose: it measures what can
be decided, not whether a model complies. `tests/check.rs` keeps the
definitions' parameter signatures equal to the registry's, so the packages
keep compiling as capabilities grow.

Run a release build over each model with `--geometry` and save one result
per model, then hand the results to the script. Keep models and results out
of the repository:

```bash
cargo build --release -p axioval-cli
for model in /path/to/models/*.ifc; do
  ./target/release/axioval check --geometry --model "$model" \
    --definitions scripts/inventory/definitions.json \
    --ruleset scripts/inventory/ruleset.json \
    --report "target/inventory/$(basename "$model" .ifc).json"
done
python3 scripts/not_evaluated_inventory.py target/inventory/*.json
```

Each argument is `[LABEL=]RESULT`; the label names the model in the output
and defaults to the file's stem, so a neutral label (`m01=…`) keeps model
names out of a table meant to be shared. `--format json` prints the same
ranking for machines, `--top N` the first N causes. `--ruleset` and
`--definitions` map rule ids of another rule set to their capabilities. The
script reads saved results only and is deterministic: the same results give
the same table byte for byte.

### How outcomes are attributed

Every not-evaluated outcome and every unmeasured object (the result's
`geometry.unmeasured`) counts once, under one cause:

- An outcome about an unmeasured object, or whose message names one, is
  caused by that object's unmeasured reason (`unmeasured: …`). A clash on a
  wall without a body is the wall's missing body, not a clash failure.
- Any other outcome is caused by its reason code, capability and message.
- An unmeasured object no outcome names still counts under its reason.

Messages are reduced to patterns: object references become `<object>`,
instance ids `#<id>` and numbers `<n>`, so one cause on many objects is one
row. A product without a body is one of two causes: `unmeasured (model
data): no shape representation`, a product with no representation at all,
which its author can fix (the result also lists it once as the integrity
warning `shape.no-representation`); and `unmeasured: no body
representation; it has <identifiers>`, a product whose representations
(`Axis`, `FootPrint`, …) include none the engine measures as a body, which
is a gap in the engine or its adapters. A face whose boundary crosses or
runs back along itself (`… profile outer ring intersects itself`, `… folds
back on itself at vertex <n>`) is model data too (#298; the result lists it
once as `shape.self-intersecting-face`), and is labelled the same way, as
is a host whose openings remove its whole body (`its openings remove its
whole body: …`, #310; listed once as `shape.voided-body`).
`--format json` marks model-data causes with `"model_data": true`. Causes are ranked by the outcomes they account for, then by unmeasured
objects, then by models affected.

## Ranking

Corpus: 15 real models, IFC2X3, IFC4 and IFC4X3, from 0.05 MB to 76 MB.
They include architectural, structural, MEP and electrical models of houses,
multi-family and office buildings, a bridge, a generated facade and small
samples.

The first ranking (#186) ran on `main` after 0.3.0, with `ifc-geometry` 0.7
and `axiolid-mesh-compile` 0.3.10. A 29 MB archive did not finish within an
hour; the 14 others recorded 28,631 outcomes and 2,535 unmeasured objects
under 23 causes.

The second ranking ran after #210, #211, #212, #213 and #214, with
`ifc-geometry` 0.8.1 and `axiolid-mesh-compile` 0.3.13. The same archive
and the 76 MB model did not finish within 30 minutes (nor within an hour);
#220 says where their time goes. The 13 others recorded **3,424 outcomes
and 577 unmeasured objects** under 21 causes: 88 % fewer outcomes and 77 %
fewer unmeasured objects than before, over one model less.

Before is the first ranking, after the second; a dash is a cause not
observed, or hidden behind another, in that run.

| # | Case | Outcomes before | Outcomes after | Unmeasured before | Unmeasured after | Models after | Cause (after) | Upstream dependency | Issue |
|---|---|---:|---:|---:|---:|---:|---|---|---|
| 1 | `no body representation` | 6,937 | 771 | 2,079 | 467 | 5 | Wholes made of parts are measured through them (#211); every remaining object has no representation at all, which is model data. Since #219 these are `no shape representation` (model data) and no longer mixed with `no body representation; it has …` | none (model data) | #219 |
| 2 | Opening without a `Body` cannot be subtracted | 216 | 691 | 36 | 36 | 3 | Reference View exports author openings as a `Reference` representation over hosts already voided; the hosts are refused, and since #212 so is every space they reach | `ifc-geometry`: take reference-only openings as applied | #218 |
| 3 | Space measurement "unavailable for the requested aspect" | 547 | 611 | 0 | 0 | 2 | Before: any unmeasured object refused every space (fixed, #212). Now: the space adapter's plan overlay refuses sliver triangles (`RepeatedVertex`) | none | #217 |
| 4 | Clash pair: proximity "unavailable for the requested object" | 19,177 | 293 | 0 | 0 | 3 | Before: the plan overlap refused slivers (fixed, #210). Now: a mesh with a zero-area triangle, mostly bodies with warped faces measured since #213 | none | #221 |
| 5 | Free floor: plan union `SelfIntersection` | 71 | 240 | 0 | 0 | 2 | An obstacle's footprint in the headroom band is refused by the overlay | none | #222 |
| 6 | Mesh compilation produced no triangles | 204 | 227 | 58 | 58 | 1 | Walls and proxies of one model whose body compiles to nothing; not yet triaged | to triage | — |
| 7 | Clash: neither body is a closed solid | 122 | 178 | 0 | 0 | 7 | Surfaces meet but neither mesh is closed, often a body of several faceted items | to triage | — |
| 8 | Containment: shared volume not measured | 97 | 144 | 0 | 0 | 4 | Furniture whose shared volume with its space is not certified | to triage | — |
| 9 | Space evidence not exact and reviewable | — | 62 | — | 0 | 1 | Spaces of one model now reach the exactness check | to triage | — |
| 10 | Space boundary: curve-bounded plane | 388 | 61 | 0 | 0 | 2 | Composite-curve boundaries are meshed (#214); the rest have too few distinct points, or rings that cross | none (model data) | — |
| 11 | Authored polygon face: rings cross | ≤ 45 | 45 | ≤ 7 | 1 | 1 | A face whose rings cross or whose hole lies outside it | none (model data) | — |
| 12 | Free floor: placement evidence not exact | 41 | 41 | 0 | 0 | 1 | A space's placement evidence is not exact | to triage | — |
| 13 | `IfcIndexedPolyCurve` profile boundary | 28 | 28 | 9 | 9 | 1 | Profile boundaries lower `IfcPolyline` and `IfcCompositeCurve` only | `ifc-geometry`: lower indexed poly curves | — |
| 14 | Space quantities "must be finite and non-negative" | — | 10 | — | 0 | 2 | The overlay refusal of case 3, read as a zero plan area | none | #217 |
| 15 | Authored polygon face not planar | 689 | — | 346 | — | — | Warped faces are tessellated within a certified bound (#213) | — | #213 |
| 16 | Containment: proximity unavailable | 47 | — | 0 | — | — | As case 4 before (#210) | — | #210 |
| 17 | Clash: proximity not finite and coherent | 22 | — | 0 | — | — | Not observed again | — | — |
| 18–23 | A free-floor obstacle that is not closed, a self-touching difference, uncertified curved bodies (non-forward extrusion, unsewn boolean), an uncomputable storey footprint, a free-floor footprint `RepeatedVertex` | ≤ 45 | 22 | ≤ 7 | 6 | ≤ 2 | | | — |

The first five cases now account for 76 % of the outcomes, and only case
2 waits on an upstream crate. Two corrections to the first ranking: case
2 is not model quality but a valid exchange the adapter does not read,
and the space refusals of case 3 now have a cause of their own, which
also turns overlay refusals into zero plan areas (case 14).

The third ranking ran on 2026-10-08 after #217, #218, #219 and #221, with
`ifc-geometry` 0.10.0 and `axiolid-mesh-compile` 0.3.14. The 29 MB and
76 MB models did not finish within 30 minutes (#220).
The 13 others are the same as in the second ranking and recorded **2,833
outcomes and 621 unmeasured objects** under 21 causes:

- Cases 2, 3, 4, 9 and 14 of the second ranking are gone: 1,667 outcomes.
  Reference View hosts are measured with their openings taken as applied
  (#218), spaces are measured past the overlay's slivers (#217), and clash
  pairs with zero-area triangles are measured (#221).
- Case 1 is now named `unmeasured (model data): no shape representation`
  (#219): 771 outcomes, 467 objects, all model data.
- Measuring the spaces of one model reaches one wall whose part cannot be
  triangulated, and since #212 every space it reaches is refused for it:
  867 outcomes from one object.
- Planar faces the triangulation refuses (no ear left, an outer ring that
  crosses, overlaps or folds back on itself) are now their own rows: 157
  outcomes and 81 objects over six models. A build from before the second
  ranking refuses the same objects, so they are not new refusals; the
  ranking had grouped them differently.

| # | Cause | Outcomes | Unmeasured | Models | Issue |
|---|---|---:|---:|---:|---|
| 1 | Whole made of parts, one part's planar face finds no ear (one wall) | 867 | 1 | 1 | — |
| 2 | No shape representation (model data) | 771 | 467 | 5 | #219 |
| 3 | Free floor: plan union `SelfIntersection` | 330 | 0 | 2 | #222 |
| 4 | Mesh compilation produced no triangles | 227 | 58 | 1 | — |
| 5 | Clash: neither body is a closed solid | 188 | 0 | 7 | — |
| 6 | Containment: shared volume not measured | 144 | 0 | 4 | — |
| 7 | Space boundary: curve-bounded boundary with too few points | 58 | 0 | 2 | — |
| 8 | Planar face finds no ear | 54 | 25 | 2 | — |
| 9–10 | Authored polygon face: outer ring intersects or overlaps itself | 86 | 42 | 3 | — |
| 11 | Free floor: placement evidence not exact | 41 | 0 | 1 | — |
| 12 | `IfcIndexedPolyCurve` profile boundary | 28 | 9 | 1 | — |
| 13–21 | Planar face ring crosses or folds back, free-floor obstacle not closed, uncertified curved bodies, uncomputable storey footprint, `RepeatedVertex` footprint, a space sliver bound | 39 | 19 | ≤ 2 | — |

The triangulation refusals (rows 1, 8, 9, 10 and part of 13–21) now
account for 1,024 outcomes, more than a third, and are the next case to
triage.

### Triangulation refusals classified (#298)

Every refused face of the 82 objects was extracted from its model, with
its IFC entity chain, and run through the same projection and clipper
(`axiolid-construct` 0.3.15, `axiolid-mesh-compile` 0.3.14). Each of the
four reasons has a single cause:

| Reason | Outcomes | Unmeasured | Models | Faces | What the faces are | Classification | Evidence |
|---|---:|---:|---:|---:|---|---|---|
| Found no ear (rows 1 and 8) | 921 | 26 | 2 | 35 | Rectangles, some with straight corners, in faceted B-reps of doors, windows, walls and one wall layer (the wall of row 1 is the whole it belongs to) | Kernel gap: once projected, the corners level with the first get `-0.0`, the clipper's lowest-vertex search orders `-0.0` before `0.0`, reads the ring as clockwise, and finds no ear. All 35 triangulate once `-0.0` is read as `0.0` | axiolid/kernel#269; `tests/check.rs` `with_geometry_a_face_crossing_itself_is_model_data` (wall `#79`) |
| Outer ring overlaps itself (row 10) | 41 | 41 | 2 | 216 | `IfcPolygonalFaceSet` faces of sanitary terminals, each an outer loop and a hole joined by one seam edge traversed both ways (seams 0.1 mm to 75 mm), nothing else touching | Kernel gap: a weakly simple ring bounding a well-defined region, the ring the clipper itself builds when it bridges a hole | axiolid/kernel#270; same test (wall `#69`) |
| Outer ring intersects itself (row 9 and part of 13–21) | 58 | 14 | 3 | 64 | Bowtie quads: a proxy's quads written in Z order (crossing 13 mm in), and twisted strip quads of pipe fittings and flexible ducts (crossing at least 10 µm in, far above rounding) | Model data | same test (wall `#49`) |
| Outer ring folds back on itself (part of 13–21) | 4 | 1 | 1 | 2 | A wall face whose loop runs out to a corner 24 mm away and back to the same point; a second face of the wall does the same 26 mm past its first corner, which rounding then reads as a crossing | Model data | same test (wall `#59`) |

The two model-data reasons are now labelled as such in the ranking
(`unmeasured (model data): …`), and the result lists each such product once
as the integrity warning `shape.self-intersecting-face`. Their reasons and
outcomes are unchanged. The two kernel gaps stay unmeasured until
`axiolid-mesh-compile` ships the fixes. Together they account for 962 of
the 1,024 outcomes.

### Fourth ranking: the whole corpus

The fourth ranking ran later on 2026-10-08, after #222 and #220, and is the
first since the first ranking to include all 15 models.

- **Large models:** with checks timed per phase, envelope declarations read only on request, and a whole's volume computed once (#220), the 29 MB model finishes in 373 s and the 76 MB model in 1,717 s.
- **Free floor:** the band footprints of obstacles with faces standing on edge are built and cached per band (#222). `plan Union failed: SelfIntersection` is gone from every model.

All 15 models recorded **12,514 outcomes and 753 unmeasured objects**
under 37 causes. The 13 models of the third ranking account for 2,826 of
those outcomes, 7 fewer than in the third ranking because #222 decides 7 more
spaces. The two large models add the rest, so the
shares shift:

| # | Cause | Outcomes | Unmeasured | Models | Issue |
|---|---|---:|---:|---:|---|
| 1 | Whole made of parts, one part's planar face finds no ear | 3,845 | 16 | 3 | #299 (axiolid/kernel#269) |
| 2 | Curved surface without a certified bound (non-forward planar extrusion) | 2,451 | 4 | 2 | — |
| 3 | Free floor: placement evidence not exact | 1,264 | 0 | 2 | — |
| 4 | Difference result touches itself along an edge (void tangent to its host's face) | 981 | 28 | 1 | — |
| 5 | Space validation: plan overlay refuses a footprint whose edges cross | 905 | 0 | 1 | — |
| 6 | No shape representation (model data) | 771 | 467 | 5 | #219 |
| 7 | Planar face: outer ring overlaps itself (keyhole) | 523 | 35 | 1 | #299 (axiolid/kernel#270) |
| 8 | Space evidence not exact and reviewable | 328 | 0 | 1 | — |
| 9 | Clash: neither body is a closed solid | 242 | 0 | 8 | — |
| 10 | Mesh compilation produced no triangles | 227 | 58 | 1 | — |
| 11 | Containment: shared volume not measured | 172 | 0 | 5 | — |
| 12 | Free floor: obstacle below the band is not a closed solid | 169 | 0 | 4 | — |
| 13 | Planar face finds no ear | 126 | 61 | 4 | #299 (axiolid/kernel#269) |
| 14–16 | Stair of separate treads; space boundary on curved faces; curve-bounded boundary with too few points | 197 | 0 | ≤ 2 | — |
| 17–37 | Smaller causes, among them `dilation: Overlay(ZeroArea)` (8) | 313 | 84 | — | — |

The two kernel gaps of #298 (rows 1, 7 and 13) now account for 4,494
outcomes, more than a third. The 76 MB model's causes (rows 2–5 and 8) are
the next to triage.

### Fourth ranking triaged

The 76 MB model's causes were each traced to their code path and rebuilt
as a synthetic case (constructed coordinates). Its plan coordinates are
about 6·10⁵ and 5.6·10⁶ m under a site turned by about 2.3°. Three of the
five causes come from that georeferencing:

| # | Cause | Outcomes | What it is | Classification | Issue |
|---|---|---:|---|---|---|
| 2 | Curved surface without a certified bound (non-forward planar extrusion) | 2,451 | A boolean with a curved operand and an extrusion against its profile normal (`ExtrudedDirection (0, 0, -1)`): an opening cut down from a slab's top, and grilles clipped by half-spaces. The exact compiler refuses the extrusion, so the boolean has no certified deviation. The one unmeasured slab has no bound, so it refuses every space of m14 | Kernel gap | #301 (axiolid/kernel#275) |
| 3 | Free floor: placement evidence not exact | 1,264 | A tessellated obstacle within reach refuses the scene. Floor slabs with arcs in their outline or openings are tessellated and span whole storeys | Engine gap | #302 |
| 4 | Difference result touches itself along an edge | 981 | Door and window openings as deep as their wall is thick, flush with both faces. Net lowering subtracts them in world coordinates, where the turned placements round the faces apart | IFC adapter gap | #303 (openbimrs/ifc#388) |
| 5 | Space validation: plan overlay refuses a footprint whose edges cross | 905 | A curved space's chord-fan triangles, about 10⁻⁴ m², wound by a shoelace over the raw coordinates, which rounds such areas to zero or the wrong sign there. The overlay does the same. One such space refused every space of its height | Kernel gap, with an engine part | #304 (axiolid/kernel#274) |
| 8 | Space evidence not exact and reviewable | 328 | Curved spaces, slabs and walls touching a space refuse its exact-only measurements | Engine gap | #305 |

For row 5 the engine now winds projected triangles by their area about a
vertex. It also measures a space only against candidates whose plan box
meets its own. On a one-storey reduction of m14 that leaves 34 of 155
refusals, now reported as `ZeroArea` until the overlay is fixed. Row 17
("Difference result touches itself at the point", 48 outcomes) is
probably row 4's case.

### Fifth ranking

The fifth ranking ran on 2026-10-09 on all 15 models after #302, #303,
#304 (engine part) and #305, with `ifc-geometry` 0.13 and `openbim-ifc`
0.19. It recorded **11,105 outcomes and 704 unmeasured objects** under
32 causes, down from 12,514 and 753.

**Resolved:**

- Free-floor placement evidence near tessellated bodies (#302): 1,264 outcomes resolved.
- Space evidence near tessellated bodies (#305): 328 outcomes resolved.
- The space overlay refusal (#304): 905 outcomes resolved.
- Openings that touched themselves on the georeferenced model (#303): 981 and 48 outcomes resolved. All 40 hosts are measured.
- `IfcIndexedPolyCurve` profile boundaries (`ifc-geometry` 0.12): 28 outcomes resolved.

**Exposed:** spaces and placements decided now reach objects that were
behind those causes.

- The "no ear" wall (#299) goes from 3,845 to 4,576 outcomes.
- Free-floor obstacles that are not closed solids go from 169 to 698.
- One wall whose polygonal half-space is bounded by an `IfcCompositeCurve`
  goes from 4 to 873 outcomes, 870 of them column distances.

| # | Cause | Outcomes | Unmeasured | Models | Issue |
|---|---|---:|---:|---:|---|
| 1 | Whole made of parts, one part's planar face finds no ear | 4,576 | 16 | 3 | #299 (axiolid/kernel#269) |
| 2 | Curved surface without a certified bound (non-forward planar extrusion) | 2,451 | 4 | 2 | #301 (axiolid/kernel#275) |
| 3 | Polygonal half-space bounded by `IfcCompositeCurve` | 873 | 1 | 1 | — |
| 4 | No shape representation (model data) | 771 | 467 | 5 | #219 |
| 5 | Free floor: obstacle below the band is not a closed solid | 698 | 0 | 5 | — |
| 6 | Planar face: outer ring overlaps itself (keyhole) | 523 | 35 | 1 | #299 (axiolid/kernel#270) |
| 7 | Clash: neither body is a closed solid | 242 | 0 | 8 | — |
| 8 | Mesh compilation produced no triangles | 227 | 58 | 1 | — |
| 9 | Containment: shared volume not measured | 172 | 0 | 5 | — |
| 10 | Planar face finds no ear | 126 | 61 | 4 | #299 (axiolid/kernel#269) |
| 11–14 | Stair of separate treads; space boundary on curved faces; curve-bounded boundary with too few points; a face whose rings cross (model data) | 242 | 1 | ≤ 2 | — |
| 15–32 | Smaller causes | 204 | 61 | — | — |

The kernel gaps of #298 (rows 1, 6 and 10) account for 5,225 outcomes,
almost half. The inventory ruleset does not measure volume, so it does
not show #306: hosts whose openings stop a rounding error short of a
face, which the volume kernel refuses far from the origin
(axiolid/kernel#276).

#### Rows 8 and 9

Every object behind rows 8 and 9 was traced through its IFC entity chain,
and every containment pair was probed for the step that left its volume
unmeasured.

| Row | Reason | Objects | Classification | Issue | Resolved |
|---|---|---|---|---|---|
| 8 | A frame modelled as a wall or proxy (44 `IfcWallStandardCase`, 14 `IfcBuildingElementProxy`, all on m15), one extrusion voided by one opening as long and as high as the frame, flush with its ends, top and bottom, and thicker: nothing is left once it is subtracted | 58 | Model data | #310 | Labelled `unmeasured (model data)` with the warning `shape.voided-body`: 227 outcomes, 58 objects |
| 9 | Furniture of several closed items that overlap or touch: one closed mesh whose triangles cross, which the volume kernel refuses as self-intersecting | m01, m07, m11, m14 | Engine gap: each item is a closed solid the kernel measures | #312 | Measured shell by shell. On the five models, 72 of the 172 outcomes are decided (71 contained, one furniture sticking out of every room it meets, reported as lying in none), and 32 straddle the ratio and say so |
| 9 | Furniture with open surface items (`IfcPolygonalFaceSet` with `Closed` `.F.`), which enclose no volume | m07, m11 | Model data: the file states surfaces | #312 | The outcome names the open surface: 52 outcomes, 53 with m03 |
| 9 | One `IfcTriangulatedFaceSet` listing each corner once per face, closed by its coordinates but not by its indices | m03 | Engine gap | #313 | Not yet: merging coincident corners closes it, but changes many public parity entries; the outcome names the open surface |
| 9 | One item of a furniture whose own triangles cross; a space prism whose cap triangles cross once placed at georeferenced coordinates | m11, m14 | Not classified yet; the space reads as a kernel gap (the same prism measures near the origin) | — | The outcome names the refused bodies: 15 outcomes |

Cases 3 and 5 are resolved upstream (`axiolid-mesh-compile` 0.3.13). A
warped authored face is now meshed, and the body is tessellated within
the width of the slab the face's corners span about its fit plane, which
`axiolid-mesh-compile` 0.3.14 reports (#213, axiolid/kernel#254, #257,
#261), or unmeasured when the face is an operand of a boolean. A curve-bounded plane bounded by a
composite or trimmed curve is meshed too (#214, axiolid/kernel#255).

Cases not observed on this corpus are listed with their capability (see
[Capabilities](./capabilities.md), [Clash](./clash.md),
[Metric routing](./metric-routing.md) and the
[adapters](./adapters.md)). They count zero here because the rule set does
not exercise them or the corpus does not contain them. Adding the rule
that reaches a case is how it enters the ranking.
