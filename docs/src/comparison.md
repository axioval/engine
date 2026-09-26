# Model comparison

`axioval_rules::compare_sessions(base, revised, request)` compares two
evidence sessions object by object. The comparison is purely semantic and
never reads geometry.

## Matching

Objects are matched by an external identity scheme, never by `ObjectId`. A
local id is whatever the source file numbered an object, and every re-export
renumbers. `ComparisonRequest::new(scheme)` names the scheme. The engine
attaches no meaning to it: an IFC session supplies `GlobalId`s under the IFC
adapter's scheme, and another source supplies its own.

Each identity ends up in one of three states:

- **Added**: it is held only by the revised session.
- **Removed**: it is held only by the base session.
- **Matched**: it is held by both. The pair then carries its differences and
  any facets that could not be compared. It is unchanged when both lists are
  empty.

## Facets

- **Kind.**
- **Classifications.** When a session registers a `ClassificationService`, its
  assignments are used; otherwise the object's own classifications are.
  Suppose one session has the service and the other does not. One side then
  lists resolved chains and the other plain codes, so every difference would be
  an artefact, and the facet is reported unresolved.
- **Carried properties.** These are the properties on the object itself,
  keyed by set and name.
- **Requested properties.** `ComparisonRequest::with_property(set, name)`
  names a property to resolve through each session's
  `PropertyResolutionServiceHandle`. Sources such as the IFC adapter answer
  properties only on request, so there is no way to list what they hold. Exact
  absence counts as a value. A resolver error, or a session without a resolver,
  leaves the property unresolved.
- **Relationships.** Targets are named by their identity in the scheme, so a
  renumbered target is not a change. A target without a unique identity leaves
  that relationship unresolved.

Values are compared exactly, except that a value equals itself (NaN included)
and the two zeros are equal.

## Nothing is dropped

- An object without an identity in the scheme is **unidentified**: it cannot
  be matched, and it may be the one that changed.
- An identity claimed by several objects of one session is **ambiguous**. This
  can happen across sources, for example with a federated copy. The identity
  matches nothing on the other side, and the object it leaves unmatched there
  is reported with the ambiguity.

`is_identical()` is true only when every identity matched and every compared
facet agreed, with nothing unidentified, ambiguous or unresolved.

## Reports

`ModelComparison::report(rule_id, severity)` projects the comparison into a
`Report`, so it can travel through any sink, BCF included:

- Additions, removals and changes each become one finding. A change names its
  base object in `related`.
- Unresolved facets, unidentified objects and ambiguous identities become
  not-evaluated outcomes.
