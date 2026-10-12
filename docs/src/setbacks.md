# Setback areas

Building codes keep the ground in front of an external wall free: a
**setback area** whose depth is a factor of the wall's height, at least a
minimum, which must lie on the wall's own parcel (or a share of the public
area beside it) and must not overlap another. The engine measures these
areas from the model, the [terrain](./terrain.md) and the
[context zones](./context-zones.md); how deep they must be and what they
may cover are the rule's statements, never constants of the engine.

Four [measured values](./derived.md) of a wall take the same parameters:

| Parameter | Meaning |
| --- | --- |
| `terrain=original\|finished` | the ground the wall's height is measured from (required) |
| `factor=<number>` | the depth per metre of the wall's height `H` (required), such as `0.4` |
| `minimum=<m>` | the least depth, whatever the height; none without it |
| `roof=<kinds>`, `roof_share=<number>` | roofs (or gables modelled apart) adding to `H`: the objects of these kinds in the wall's source whose footprint overlaps the wall's add the share of their rise above the wall's top; one is refused without the other |

- `setback_depth` is `factor × H`, at least `minimum`, where `H` is the
  wall's top above the mean terrain along its outline
  (`above_terrain;terrain=…;of=top`) plus `roof_share` of the greatest rise
  of a roof over it above the wall's top. A roof that may or may not stand
  over the wall raises only the most. `H` below the ground counts as zero.
- `setback_area` is the area of the setback: the wall's footprint swept
  straight out by the depth along every plan direction its steep faces look
  outside, less the footprint itself.
- `setback_outside_area;…;zone=<id>` is the part of it outside a context
  zone, such as the parcel with half the street beside it, under every
  boundary the zone's accuracy allows.
- `setback_overlap_area;…;others=<objects>` is the part of it the setbacks
  of the `others` (each measured with the same parameters) overlap,
  united. Walls of other buildings, and of other models placed in the same
  frame, are compared alike; the value's evidence names the walls whose
  setbacks may overlap (`overlapped-by=…`), and a rule reading it with
  bound arguments relates them.

A rule then states the requirement: `setback_outside_area;terrain=original;factor=0.4;minimum=3;zone=P1`
at most `0 m²`, or `setback_overlap_area;…;others=IfcWall` at most
`0 m²`. A finding names the area beyond the parcel or overlapped.

## Which way a wall looks out

The outward side comes from the facade-area service's faces looking
outside (`FacadeAreaService::measure_facade_faces`), as `azimuth` reads
them: one direction must surely hold more area than any other may. A wall
looking out both ways (no room declared behind it) or several ways none
surely largest is not evaluated, never given a side by a guess. Its steep
faces (normal nearer the horizontal than the vertical) then each give a
plan direction to sweep along: a wall's main face, and its end at a corner
where it stands free. A face wholly outside makes its direction sure; a
face only partly outside makes it possible, adding only to the most. A
wall no steep face of which looks outside, such as an internal wall, has
no setback, exactly. A tessellated wall is not evaluated: the directions it
looks along are approximate.

## Intervals

The terrain is known to its stated accuracy, so `H` and the depth are
intervals, and so is every area: the least sweeps the sure directions by
the least depth, the most every direction by the most depth. A zone's
boundary is bracketed as for the zone values (the stated zone less a band
of its accuracy, and with it), so a setback reaching the boundary within
that accuracy is never surely inside, and an overlap is surely there only
between sure walls at their least depths. Nothing is defaulted: no terrain,
a wall beyond it, an unplaced zone or an unmeasurable compared wall leaves
the value not evaluated. A check reading a setback value without the ground
it names warns `terrain-not-supplied`, and one reading
`setback_outside_area` without the zone warns as zone values do.

## The seam

`PlanAreaService::measure_setback` answers a `SetbackRequest`: a
`SetbackStrip` (the wall, its `SetbackDirection`s, each a unit plan vector
and whether surely outward, and the depth interval), optionally a zone's
`PlanPolygon`s and accuracy (`with_zone`), and optionally other walls'
strips, each surely or possibly compared (`with_others`, never the wall
itself). The answer, a `SetbackArea`, gives the setback's polygons at the
least and at the most, its area, the part outside the zone and the part
overlapped where asked, and the compared walls whose setbacks may overlap
it; its evidence is exact exactly when every interval is a point. The
default refuses, the handle refuses an answer about another wall, with a
part not asked for or naming a wall not compared, and `GroupAreas` refuses
derived groups. The Axiolid adapter sweeps each plan triangle of the
footprint along the direction (the hull of the triangle and its translate)
and unites, subtracts and intersects with the plan overlay, as a grown
footprint and a zone's parts are measured (see
[Adapters](./adapters.md)).
