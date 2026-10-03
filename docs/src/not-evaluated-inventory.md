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
row. Causes are ranked by the outcomes they account for, then by unmeasured
objects, then by models affected.

## Ranking

Corpus: 15 real models, IFC2X3, IFC4 and IFC4X3, from 0.05 MB to 76 MB.
They include architectural, structural, MEP and electrical models of houses,
multi-family and office buildings, a bridge, a generated facade and small
samples. One 29 MB archive did not finish meshing within an hour and is
left out. The 14 that ran recorded 28,631 outcomes and 2,535 unmeasured
objects under 23 causes. The run used `main` after 0.3.0, with
`ifc-geometry` 0.7 and `axiolid-mesh-compile` 0.3.10.

| # | Case | Outcomes | Unmeasured objects | Models | Cause | Upstream dependency | Issue |
|---|---|---:|---:|---:|---|---|---|
| 1 | Clash pair: proximity "unavailable for the requested object" between two meshed bodies | 19,177 | 0 | 10 | The plan overlap of the pair's projected triangles is refused by the overlay (`RepeatedVertex`) because sliver triangles are not filtered at its tolerance | none | #210 |
| 2 | `no body representation` | 6,937 | 2,079 | 11 | 78 % are wholes decomposed into parts (`IfcRelAggregates`) whose parts carry the body | none | #211 |
| 3 | Authored polygon face not planar | 689 | 346 | 2 | The mesh compiler refuses a faceted face whose corners leave its plane | `axiolid-mesh-compile`: triangulate with a certified bound | #213 |
| 4 | Space measurement "unavailable for the requested aspect" | 547 | 0 | 5 | Any unmeasured role object or storey anywhere refuses every space (`complete()`) | none | #212 |
| 5 | Space boundary "is not a curve node" | 388 | 0 | 3 | A curve-bounded plane bounded by a composite curve lowers to a curve relation the compiler does not accept | `axiolid-mesh-compile`: resolve curve relations as boundaries | #214 |
| 6 | Opening without a body cannot be subtracted | 216 | 36 | 3 | The model's opening has no `Body`; the host is refused rather than measured without it | none (model quality) | — |
| 7 | Mesh compilation produced no triangles | 204 | 58 | 1 | Walls and proxies of one model whose body compiles to nothing; not yet triaged | to triage | — |
| 8 | Clash: neither body is a closed solid | 122 | 0 | 6 | Surfaces meet but neither mesh is closed, often a body of several faceted items | to triage | — |
| 9 | Containment: shared volume not measured | 97 | 0 | 3 | Furniture whose shared volume with its space is not certified | to triage | — |
| 10 | Free floor: plan union `SelfIntersection` | 71 | 0 | 2 | The overlay refuses a self-intersecting obstacle footprint | `axiolid-overlay` | — |
| 11 | Containment: proximity unavailable | 47 | 0 | 3 | As case 1, for containment pairs | none | #210 |
| 12 | Free floor: placement evidence not exact | 41 | 0 | 1 | A space's placement evidence is not exact | to triage | — |
| 13 | `IfcIndexedPolyCurve` profile boundary | 28 | 9 | 1 | Profile boundaries lower `IfcPolyline` and `IfcCompositeCurve` only | `ifc-geometry`: lower indexed poly curves | — |
| 14 | Clash: proximity not finite and coherent | 22 | 0 | 1 | A measured proximity is rejected as incoherent | to triage | — |
| 15–23 | Rings that cross, a free-floor obstacle that is not closed, storeys sharing an elevation, free-floor footprint `RepeatedVertex`, a self-touching difference, uncertified curved bodies (non-forward extrusion, unsewn boolean), an uncomputable storey footprint, an undecided duplicate | 45 | 7 | ≤ 2 | | | — |

Cases 1, 2 and 4 alone account for 93 % of the outcomes, and none of them
waits on an upstream kernel. Case 2 also feeds case 4: a slab made of parts
leaves every space of its model refused.

Cases not observed on this corpus are listed with their capability (see
[Capabilities](./capabilities.md), [Clash](./clash.md),
[Metric routing](./metric-routing.md) and the
[adapters](./adapters.md)). They count zero here because the rule set does
not exercise them or the corpus does not contain them. Adding the rule
that reaches a case is how it enters the ranking.
