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
