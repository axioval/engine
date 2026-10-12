# Context zones

Planning law judges a building against plan areas a model does not carry:
the parcel it stands on, the building limits and building lines of a
zoning plan, the area of a use. The engine reads these as **context
zones**, supplied beside the model for a check (`check --zones`, see
[Command line](./cli.md#context-zones-supplied-beside-the-model)), and
relates objects to them through [measured values](./derived.md): how much
of a footprint lies inside or outside a zone, how far a footprint lies
from a zone's line, and the areas the plot ratios GRZ and GFZ compare.

A zone is a statement of whoever supplied it, never the model's. Every
value read from one cites the zone, the SHA-256 of the file it came from
and the feature's position
(`axioval:zone;id=<id>;kind=<kind>;sha256=<digest>#feature=<n>`).

## What a zone states

Each zone has:

- an **id**, unique among the zones supplied to a check, which measured
  values name (`zone=P1`); it may not be blank, start with `@`, or hold
  `;`, `=`, `#`, `|`, `,` or whitespace;
- a **kind**, a lowercase word: `parcel`, `zone` (the area of a use),
  `building-limit`, `building-line`, or any other word a reader assigns;
- its **geometry**: polygons that do not overlap (each with holes), or a
  line;
- the **accuracy** of its boundary in metres: the true boundary lies
  within that distance of the stated one. It must be stated; a zone
  without one is refused, never read as exact;
- its **provenance**: the file's name and SHA-256 and the feature's
  position in it.

The engine holds these as `ContextZones` (one file: origin, digest,
reference system and its `ContextZone`s) and a run's `SuppliedZones`
(every file, by id). A reader of another exchange, such as a cadastral
export (#319) or a zoning plan (#321), produces the same `ContextZones`
with `ContextZones::new`, so every measured value reads zones one way
whatever they were read from.

## The file

The one input format for now is a GeoJSON `FeatureCollection` that names
its reference system in the member `crs`:

```json
{
  "type": "FeatureCollection",
  "crs": { "type": "name", "properties": { "name": "axioval:model" } },
  "accuracy": 0.05,
  "features": [
    { "type": "Feature", "id": "P1",
      "properties": { "kind": "parcel" },
      "geometry": { "type": "Polygon",
                    "coordinates": [[[0, 0], [30, 0], [30, 20], [0, 20], [0, 0]]] } },
    { "type": "Feature", "id": "B1",
      "properties": { "kind": "building-limit", "accuracy": 0 },
      "geometry": { "type": "Polygon",
                    "coordinates": [[[0, 0], [20, 0], [20, 15], [0, 15], [0, 0]]] } },
    { "type": "Feature", "id": "L1",
      "properties": { "kind": "building-line" },
      "geometry": { "type": "LineString", "coordinates": [[0, 15], [30, 15]] } }
  ]
}
```

- `crs` is `axioval:model` for the model's own plan coordinates in metres,
  or the name of a map coordinate reference system (`EPSG:25832`,
  `urn:ogc:def:crs:EPSG::25832`), whose coordinates are in that system's
  unit. A collection naming none is refused: GeoJSON's default, longitude
  and latitude, is never placed in a model.
- The id is the feature's `id` (text or a number), or its property `id`.
- `kind` is a property; `accuracy` is the feature's property, else the
  collection's member `accuracy`.
- The geometry is a `Polygon`, a `MultiPolygon` or a `LineString`; a third
  coordinate is ignored. A ring needs three points enclosing an area, a
  line two points, and no point may repeat beside itself.
- Other properties are kept in the file and never read. Stated values a
  zone carries, such as a zoning area's GRZ, are #321's.

## Placing a zone in a model

Every source states its geometry in its own model coordinates. A zone
stated in `axioval:model` is used as stated. A zone stated in a map system
is placed in a source's coordinates only through the map conversion that
source states ([`CoordinateSystemService`](./services.md)), and only when
the transform is established:

- the source states a map conversion, to a reference system of the same
  name (EPSG codes compared by their number);
- the map unit is known exactly;
- the source's world frame is stated as the identity;
- the conversion's scale is within 0.001 of one, so it cannot hold a unit
  conversion.

Otherwise the value reading the zone is not evaluated, naming what is
missing; a zone is never placed by a guess. A placed zone's accuracy grows
by the transform's rounding, so a zone placed through a map conversion is
never exact.

## Measured values

| Name | Of | Value |
| --- | --- | --- |
| `zone_inside_area;zone=<id>` | each object | the area of its footprint inside the zone |
| `zone_outside_area;zone=<id>` | each object | the area of its footprint outside the zone |
| `zone_distance;zone=<id>` | each object | the least plan distance from its footprint to the zone's line, or to its boundary, zero where they meet |
| `zone_area;zone=<id>` | each object (its source places the zone) | the zone's own area |
| `zone_footprint_area;zone=<id>;elements=<objects>` | the project | the area of the union of the elements' footprints inside the zone, overlaps counted once |
| `zone_floor_area;zone=<id>;elements=<objects>` | the project | the sum of each element's footprint inside the zone, so floors stacked above one another each count |

`count_within`, `sum_within`, `min_within`, `max_within` and
`mean_within` take `zone=<id>` as their reach: the count of, or a value
aggregated over, the counterparts lying in the zone, such as the trees
on a parcel (see [the values within a reach](./derived.md)). A
counterpart lies in it by its footprint as `zone_inside_area` and
`zone_outside_area` tell, or, with no footprint of any area, by its
placement point; one straddling the boundary may or may not count.

`zone` may name a text parameter of the rule instead (`zone=@parcel`), so
one rule package serves every project; `elements` takes source kinds or a
selector parameter, and the elements must lie in one source.

- **Inside, crossing or outside.** A footprint lies inside a zone where
  `zone_outside_area` is zero, outside it where `zone_inside_area` is zero,
  and crosses it otherwise; the area outside is the measure of how far it
  crosses. A building limit is kept with `zone_outside_area <= 0 m²`, or
  within a stated allowance of area.
- **A building line.** A facade is on a building line where
  `zone_distance` is at most the allowed tolerance.
- **GRZ** is `zone_footprint_area` of the parcel's buildings over its
  `zone_area`: a `divide` expression. **GFZ** is `zone_floor_area` over
  `zone_area`, the elements being what states each storey's floor area,
  such as every storey's floor slab.

Areas are intervals sure to hold the exact value under every boundary the
zone's accuracy allows: the area surely inside is measured against the
zone less the band within its accuracy of its boundary, the area possibly
inside against the zone with that band, and the outside the other way
round. A tessellated footprint widens them by its chord band. A footprint
straddling a zone's boundary within its accuracy therefore has an area
outside from zero up, and is never surely inside: a rule requiring none
outside is not evaluated for it, never passed.

An aggregate counts an element the selection surely picks towards both
ends, and one it leaves undecided only towards the most it may be.

What leaves a value not evaluated, never defaulted: no zones supplied, no
zone of that id, a zone that cannot be placed, a line where an area is
asked, a footprint that cannot be read (a body that could not be measured,
an overlay refusal), and elements in several sources.

A building 10 × 8 m crossing its building limit by 0.3 m measures 2.4 m²
outside it; two buildings of 80 m² and 30 m² on a parcel of 600 m² cover
110 m², a GRZ of 0.18.

## Services

The areas come from `PlanAreaService::measure_zone_area`
(`ZoneAreaRequest`: the objects, sorted, of one source; the zone's
`PlanPolygon`s in the model's coordinates; its accuracy) answered by a
`ZoneArea` (the united footprint, the part inside and the part outside,
each an interval, exact exactly when every one is a point). The default
refuses, and the handle refuses an answer about other objects. The
[Axiolid adapter](./adapters.md) answers it with the plan overlay. The
distance comes from `ProximityService::measure_line_distance` in plan, one
segment at a time, each end boxed by the zone's accuracy.

## Not yet

- Selecting "the parcel the building stands on" or "the zoning area it
  lies in" by kind, and a zone's neighbours, are the readers' (#319,
  #321); rules name a zone by id or by a rule parameter for now.
- Aggregating other values over the counterparts within a zone (trees per
  parcel) is #360's zone reach, which reads the containment measured here.
