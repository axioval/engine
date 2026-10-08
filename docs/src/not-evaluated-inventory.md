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
is a gap in the engine or its adapters. `--format json` marks the first
with `"model_data": true`. Causes are ranked by the outcomes they account for, then by unmeasured
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

Case 2's upstream entry point is published (`ifc-geometry` 0.10,
openbimrs/ifc#351), and since #218 `--geometry` takes an IFC4 or IFC4X3
opening whose every representation is `Reference` as already applied, so
its host is measured from its `Body` as authored. The rankings above
predate it; the next run shows whether the cause is gone from the three
models.

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
