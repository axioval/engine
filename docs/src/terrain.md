# Terrain as context

Heights above the ground decide building classes, setback depths and
access: the floor level of the highest storey above the mean terrain, the
wall height at the facade line. The ground rarely comes with the building
model. It comes from a digital terrain model kept beside it, so the engine
reads a terrain as **context**: geometry supplied for a check, never part of
any source's objects.

## Original and finished ground

A terrain is at most one surface of each `TerrainKind`: the **original**
ground, before construction, and the **finished** ground, after it. Both may
be supplied, and each is served on its own: a rule names the ground it
measures against, and a ground not supplied leaves what reads it not
evaluated (`no finished terrain is supplied`), never measured against the
other one.

## The service

`TerrainService` is the seam (`crates/engine/core/src/terrain/`):

- `surfaces` states each surface served: its kind, its name in the source,
  the coordinates it is given in (`TerrainFrame::Model` or `Map`), the map
  coordinate reference system the source names, its vertical accuracy and a
  locator, `axioval:terrain;kind=<kind>;sha256=<digest>#surface=<name>`.
- `measure_elevation` answers a `TerrainPointRequest` (a kind and a plan
  point in the surface's coordinates) with a `TerrainElevation`.
- `measure_mean_elevation` answers a `TerrainOutlineRequest` (a kind, closed
  plan rings and how far the true outline may lie from them) with the mean
  elevation along the rings, weighted by plan length. Its default refuses,
  never answering with one point's elevation.

`TerrainServiceHandle` refuses a request for a kind the service does not
serve (`TerrainError::NotSupplied`) before asking it, and an answer about
another kind or carrying another accuracy than the surface states.
`TinTerrain` serves triangulated surfaces (`TerrainSurface`) held in memory,
at most one of each kind. A host gives the runtime its terrain with
`Runtime::with_terrain`, which installs it in every run, replacing any copy
registered among a session's services.

## Accuracy is part of the evidence

Every elevation is an interval: the surface's own elevation widened on both
sides by its stated vertical accuracy and by the rounding of the
interpolation. `TerrainElevation::try_new` refuses an interval narrower than
twice the accuracy, and an exact answer with any accuracy above zero. The
accuracy is a bound the host states on how far any elevation of the surface
may lie from the true ground (state a multiple of a standard deviation, not
the deviation itself, if the survey gives one).

An accuracy is never assumed. LandXML states none for a surface, so the
reader takes it from its caller, and the command line requires it with every
file. Only a stated zero makes an elevation exact.

## Plan points and outlines

The elevation at a plan point is the plane of the triangle holding it.
Points on an edge or a vertex are held by every triangle meeting there,
which agree on a proper TIN; triangles that hold one plan point at two
elevations fold the surface over it (`TerrainError::Folded`), refused rather
than one picked. Triangles without plan area (vertical faces) hold no point.

A point no triangle holds is refused by name
(`TerrainError::Outside`, naming the point and the surface: the plan point
(60, 0) lies outside the original terrain surface `Ground`), never answered
with the elevation of the nearest edge.

The mean along an outline is exact for the surface: each ring edge is cut
where it crosses a triangle edge, the elevation is linear between two cuts,
and the pieces are summed by the trapezoid rule and divided by the rings'
plan length. A piece whose midpoint no triangle holds leaves the outline
partly outside the surface, and the mean is refused by that point. An
outline stated within a deviation `d` of the true one (a tessellated body's
chord deviation) widens the mean by `d` times the steepest gradient of the
triangles it crosses.

## LandXML surfaces

`TerrainSurface::read_landxml` reads one surface of a LandXML 1.x document:

- the `Surface` with the requested name, or the document's only one (a
  document with several needs the name);
- its `Definition`, which must be `surfType="TIN"`;
- its points, `Pnts/P`, each `northing easting elevation`, read as
  `x = easting`, `y = northing`;
- its faces, `Faces/F`, three point ids each; an invisible face (`i="1"`)
  is left out;
- the linear unit `Units/Metric` or `Units/Imperial` states (metre,
  kilometre, decimetre, centimetre, millimetre, foot, US survey foot, inch);
  a file stating none, or another unit, is refused;
- the coordinate reference system `CoordinateSystem` names (`EPSG:<code>`
  from `epsgCode`, else its `name`), kept for a map surface only.

A file that is not UTF-8, declares a document type, names a point a face
lacks, states a point twice or not as three numbers is refused. The SHA-256
of the file's bytes is part of the surface's locator.

## Coordinate systems

A surface is given either in the models' own coordinates (`Model`) or in
map coordinates (`Map`), and a service answers only in its own coordinates.
Placing a model's point on a map surface is its reader's, through the
stated map conversion of the model's source, and never guessed.

## Heights above the terrain

The measured values `above_terrain` (of a point, an object's bottom or top,
or a level) and `mean_terrain_height` (the mean terrain elevation along a
building's outline) read the terrain; see
[Heights above the terrain](./derived.md#heights-above-the-terrain). The
outline is traced by `PlanAreaService::measure_footprint_outline`
(`FootprintOutline`: the outer rings of the union of objects' footprints,
the chord deviation of the bodies they come from, exact evidence exactly
when that is zero), whose default refuses rather than answering with a
bounding box. A model point is placed on a map surface through its
source's stated map conversion (`MapPlacement`, which context zones
share), and every refusal to place it (no conversion, an inexact map unit,
a world frame not stated as the identity, a scale more than 0.1 % from
one, another named reference system) leaves the value not evaluated.

The [setback values](./setbacks.md) read a wall's height above the terrain
through `above_terrain`, so the terrain's accuracy widens their depth and
areas too.

## Command line

`axioval check --terrain KIND@ACCURACY=FILE[#SURFACE] --terrain-in model|map`
supplies a surface (see [Command line](./cli.md#terrain-beside-the-model)).
