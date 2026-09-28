# Refining outcomes

A capability decides what is found and what could not be evaluated. A rule
instance may additionally say how those outcomes are reported, and a host may
ask the runtime to report more about them. Both apply after the capability
ran, the same way for every capability, so no capability grows a parameter
for them and no existing definition changes its signature.

Every refinement is optional. A rule instance declaring none, run by a host
asking for nothing more, produces the report it always did, byte for byte.

## Severity bands

A finding of a value missing its bound may be graded by how far it misses.
A rule instance declares `severityBands`, ascending thresholds of the
relative deviation, each with a severity:

```json
"severity": "error",
"severityBands": [
  {"below": 0.05, "severity": "info"},
  {"below": 0.2, "severity": "warning"}
]
```

The relative deviation is `|value - bound| / |bound|` for the bound the value
misses. A deviation below `0.05` is `info`, from `0.05` below `0.2` is
`warning`, and from `0.2` on keeps the rule's `severity`. A plan area of
97 m² against a 100 m² minimum is 3 % short and `info`; one of 70 m² is 30 %
short and `error`. A bound of zero makes any miss infinitely large.

Capabilities measure values as intervals, so a deviation is an interval too.
One that straddles bands takes the most severe band it may reach, and the
finding's message then ends with the deviation and the band it was graded
by. Grading never changes which findings exist, only their severity.

A capability reports the deviation of each finding it can
(`CapabilityEvaluation::push_graded_finding`, with an `axioval_engine::Deviation`
made by `Deviation::below` or `Deviation::above`) and declares
`RuleCapability::grades_deviation`. A finding with no bound to miss, such as
"no limit defined", keeps its severity. Compilation refuses bands on a
capability that reports no deviation, and bands that are not finite,
positive and strictly ascending (`EngineError::InvalidRefinement`).

These capabilities grade their findings:

| Capability | Deviation |
|---|---|
| `plan-area`, `area-ratio` | the area or ratio against the bound it misses |
| `related-count` | the count (with undecided objects, an interval) against its bound |
| `keyed-limit` | the measured quantity against the row's bound; sill heights by the worst floor |
| `table-allocation` | a row's count against its count, its summed area against the tolerance band |
| `space-distance` | the nearest distance against the row's minimum or maximum |
| `opening-zone` | a distance from the host's ends, edges or flanges, or from another opening, against its bound; an opening outside every allowed zone by the zone it misses least |
| `space-validation` | an unallocated floor region's area against `maximum_unallocated_area_square_metres` |
| `stair-geometry`, `ramp-geometry` | every numeric check: risers, goings, step lengths, widths, rises, headroom, landings, handrail heights, extensions and gaps, slope limits (the nearest row) |

Several failing values in one finding (risers of one flight) grade by the
one missing most.

## Severity overrides

A finding may take another severity depending on the objects it involves:
a clash is a warning in general but an error when a load-bearing wall is
involved. A rule instance declares `severityOverrides`, each a selector and
a severity:

```json
"severity": "warning",
"severityOverrides": [
  {"selector": {"kind": "property", "propertySet": "t.Pset", "property": "t.LoadBearing",
                "operator": "equals", "value": {"type": "boolean", "value": true}},
   "severity": "error"}
]
```

An override holds for a finding when its selector selects the finding's
subject or any of its related objects. Overrides are tried in order and the
first that holds decides, over the severity the rule and its bands gave; the
property facts the selector read are cited in the finding's evidence. An
override that cannot be decided for an involved object (its property cannot
be read, its concept is unbound) may or may not hold: when that could change
the severity, the finding is reported not evaluated, naming the severities it
may take and why, never given the rule's severity by default. When every
possibility agrees, the severity stands.

Overrides are applied by the host's outcome refiner
(`axioval_engine::OutcomeRefiner`, installed with
`CapabilityRegistry::with_refiner`), since evaluating a selector reads the
model. `axioval_rules::register_builtins` installs `axioval_rules::Refiner`.
A rule declaring overrides against a registry without a refiner, or naming a
concept no definition package declares, fails compilation.

## Nested categories

Reviewers file findings under categories: by fire rating and use, by the
type of the space a window adjoins. A rule instance declares `categories`,
an ordered list of properties, each read on the finding's subject or, with
a `path`, on the objects the path reaches from it:

```json
"categories": [
  {"propertySet": "t.Pset", "property": "t.FireRating"},
  {"propertySet": "t.PsetSpace", "property": "t.Use", "path": ["axioval:derived.adjacent-space"]}
]
```

Each level heads the finding's message in brackets, outermost first:
`[F90] [Office] sill height above ...`. Several objects reached with
different values share one heading (`[Lab, Office]`); no value (absent,
null, blank, or nothing reached) is `[-]`, so every level keeps its place.
The values read are cited in the finding's evidence. A property or path
that cannot be read leaves the finding not evaluated, never filed under the
wrong heading. A finding about a source or the project has no subject and
is not categorised. A level's property must be a declared concept (or a
reserved set such as `axioval:attributes`); paths take the steps of a
`related` selector, derived relationships included.

The levels are also kept as data: the finding's `categories` lists each
level's heading text outermost first (`["F90", "Office"]`, `-` for no
value), so a sink can file it without parsing the message. BCF labels the
topic `Category: F90 / Office`.

Categories apply to every capability. `property-comparison` and
`property-requirements` keep their own `category_property`, which heads one
property and adds nothing for no value; declared together, the rule's
categories come first.

## Locations

Reviewers group, filter and export findings by storey and space. A host may
ask the runtime to locate every finding and not-evaluated outcome
(`Runtime::with_locations(LocationPolicy)`); a package never does. The
policy names the method and what storeys and spaces are in the sources' own
vocabulary: object kinds, a containment path and the property naming a
place.

| Method | Storeys | Spaces |
|---|---|---|
| `Storeys` | the nearest storeys up the containment path | none |
| `Containers` | as `Storeys` | the nearest spaces up the containment path |
| `Geometry` | as `Storeys` | the spaces whose body contains or meets the object (`axioval:derived.contained-in-space`) |

The containment path's steps are climbed in any order, stopping at each
storey or space reached; an object that is itself a storey or space lies in
itself, and one placed in no storey lies on its spaces' storeys. A finding is
located by its subject and every related object together (a duct–wall clash
by both), a not-evaluated outcome by its object; an outcome naming no object
has no location.

```json
"location": {
  "storeys": [{"id": {"source": {...}, "local_id": "#100"}, "name": "Level 1"}],
  "spaces": [{"id": {"source": {...}, "local_id": "#39"}, "name": "101"}]
}
```

`Finding::location` and `NotEvaluated::location` are `None` unless the host
asked, and absent from the wire then, so a report without locations
serializes byte for byte as before. A location decides nothing about the
outcome. When part of it cannot be derived (a relationship refused, no
derived relationships without geometry), `unresolved` says why and the lists
may be incomplete; readers filtering by location keep such an outcome.
Locating needs the registry's outcome refiner; without one the run fails
(`EngineError::MissingRefiner`).

Locations are not part of a finding's identity: the BCF sink's GUID key
ignores them, so a finding keeps its GUID whether or not it was located.

## Rule status

A report of findings cannot tell a rule that selected nothing from one that
passed on 500 objects. A host may ask for one summary per rule
(`Runtime::with_rule_summaries()`), reported in `Report::rules`, by rule id:

```json
"rules": [
  {"rule_id": "columns-named", "checked": 0, "failed": 0, "not_evaluated": 0,
   "status": "nothing_selected"},
  {"rule_id": "walls-named", "checked": 10, "failed": 2, "not_evaluated": 0,
   "status": "failed"}
]
```

- `checked` is the decided selection: the objects the rule's applicability
  selector surely selects. An object it cannot decide is not counted; its
  capability reports it not evaluated.
- `failed` and `not_evaluated` count the distinct objects the rule's
  findings and not-evaluated outcomes are about, after refinement. An outcome
  about a source or the project counts no object.
- `status` is `failed` with any finding, else `not_evaluated` with any
  not-evaluated outcome (at any scope), else `nothing_selected` when nothing
  was checked, else `passed`. A rule compiled but not executable is
  `not_evaluated` with nothing checked.

Counting a selection evaluates the selector, so it needs the outcome refiner
(`OutcomeRefiner::selected`); without one the run fails. Without the option
`rules` is empty and omitted, so reports serialize as before. Rule status
never changes a finding or the CLI's exit status.
