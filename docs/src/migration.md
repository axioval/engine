# Capability migration

A box is checked only when the Axioval implementation is the production owner, the legacy runtime consumes it, and parity evidence passes.

## Current capability waves

- **Property comparison:** the bounded published `0.1.0` slice is consumed by the legacy runtime and recorded in the machine ledger as `in_progress`; unsupported operators and modes remain outside the cutover.
- **Required property:** `axioval:capability.property-required` is published in the `0.1.1` facade and tested in the engine. It is deliberately not marked migrated until the legacy production checker executes it with exact source evidence.

## Shared runtime

- [ ] Normalized package contract and strict binder
- [ ] Capability registry/compiler
- [ ] Deterministic runtime/reporting
- [ ] Source/evidence sessions and caches
- [ ] Selection engine and quantifiers
- [ ] Pairwise/spatial engine
- [ ] Circulation/path/free-space graph
- [ ] Exact recognizers
- [ ] Typed property sources

## Capability families

- [ ] Information and property
- [ ] Relationship and assignment
- [ ] Space and topology
- [ ] Spatial clearance/distance
- [ ] Building
- [ ] Accessibility
- [ ] Life safety
- [ ] Federation and comparison

## Host rewiring

- [ ] `spec` normalizes to Axioval contracts
- [ ] `checker` executes Axioval plans
- [ ] CLI and Python use Axioval reports
- [ ] IFC parsing/modeling uses `openbimrs/ifc`
- [ ] Geometry evidence uses `axioval-axiolid`
- [ ] ICDD uses `openbimrs/icdd` plus `axioval-icdd`
- [ ] BCF/report adapters consume generic findings
- [ ] Duplicate engine/model/geometry ownership removed

## Required evidence per row

Record the Axioval tests, legacy tests, oracle corpus, discrepancy count, performance measurement, cutover commit and rollback switch. Unsupported behavior remains unchecked and documented; it is never represented by an empty module or unconditional success.

## Retired rule plans

`axioval-spec` carried typed plans (`*PlanSpec`) for rule families that the
registered capabilities now express directly. A plan that only restated a
capability's parameters under another shape was removed rather than given a
lowering: a lowering would bind `axioval-spec` to `axioval-ir`, which the spec
crate's dependency contract forbids, and mapping a host's own rule shapes onto
capability rulesets is a backend binding that belongs to that host. Authors
write these checks as rules over the capabilities below; each row names the
capability and, where one exists, the section of [Capability
model](./capabilities.md) that shows the composition.

| Removed plan (`CheckSemantics` variant) | Replaced by |
|---|---|
| `RelativeCountPlanSpec` (`RelativeCount`) | `relative-count`: `provided_selector` and `required_selector` for the two entity types, ratio mode with `provided_unit`, `required_unit` and `operator`, or table mode with `table`, `additional_provided` and `additional_required`. Grouping per storey is an anchor selection of storeys with a traversal; the whole model is one anchor per source. |
| `PropertyComparisonPlanSpec` (`PropertyComparison`) | `property-comparison`: `component_mode` `checked`, `related` (relationship or `path`), `shared`, or the container modes `same_space` and `same_building`, whose `container_selector` also names walls or storeys; quantifiers `count` and `sum`; `category_property` for categorization; constant, property, text-list and range targets, a range bounded by numbers, quantities, dates, date-times or properties of the checked object. |
| `ManualIssuePlanSpec` (`ManualIssues`) | `manual-issue`, one rule per issue row, its `title`, `category` and `description`; the row's scope is the rule's selection. |
| `LayerAgreementPlanSpec` (`LayerAgreement`) | `selector-conformance`: the component and construction type in the rule's selection, `axioval:presentation` `Layer` `oneOf` the agreed layers with `quantifier: all` as the `requirement`. |
| `ModelArchitecturePlanSpec` (`ModelArchitecture`) | One rule per flag. Empty storeys, orphan doors and windows, and elements not directly contained: `related-count` with `minimum: 1`. Storey elevations and names repeated within a building: `unique-value` with a `relationship` to the building. A single site: `object-count` with `maximum: 1`. Layer thickness against the body: `body-extent`. Polygon count: `triangle-count`. Doors on their host's storey: `same-container`. Space-boundary coverage: `space-boundary-coverage`. GlobalId uniqueness: the IFC adapter's `identity.duplicate-global-id` diagnostic within a file, `unique-value` with `across_sources` across models. Disciplines: `discipline` selectors. See "Model quality". Door swing direction: `door-swing`. Site geometry presence: `property-required` on `axioval:body.Count` over the sites; a site whose representations state no body (none at all, or only a footprint or an axis) is a finding, and one whose body cannot be read is not evaluated. |
| `BuildingStoreyPlanSpec` (`BuildingStorey`) | The compositions of "Storey metrics": `level-spacing` for storey heights (`minimum`, `maximum`, `consistent`, `ignore_lowest`, `ignore_highest`), `area-ratio` for the net-area and empty-area ratios and for window-to-wall ratios per storey and per building (`measure: facade`), `plan-area` with `measure: facade` for external-wall area. |
| `RequiredComponentsPlanSpec` (`RequiredComponents`) | `object-count` per source, or with `across_sources` over the project, one rule per row or one rule over an `anyOf` of the rows; the source's discipline through `disciplines` (only the models of those disciplines are counted) or a `discipline` selector. A required construction type or classification is `property-required` or `selector-conformance` over the same selection. |
| `SpacesInDerivedGroupsPlanSpec` (`SpacesInDerivedGroups`) | `group-composition` with `ungrouped_selector`, or `related-count` with `minimum: 1` from each space along `axioval:derived.overlapping-group-space`; skipped spaces are left out of the rule's selection. |
| `SpaceGroupContainmentPlanSpec` (`SpaceGroupContainment`) | `group-composition`, a `requirements` table of member patterns and counts, with `group_key` for per-group rows. |
| `FireCompartmentAreaPlanSpec` (`FireCompartmentArea`) | `keyed-limit` with `quantity: plan-area`, keyed on the building's fire rating, the compartment's use class and the sprinkler flag. |
| `StoreyNameSequencePlanSpec` (`StoreyNameSequence`) | `name-sequence` from each building, `order` the storey `Elevation`, with `first` and `increment`; `order_fallback: placement_height` orders a storey without `Elevation` by its placement height. |
| `ModelComparisonPlanSpec` (`ModelComparison`) | `model-comparison`: the models by discipline (`base`, `revised`) for `first_model_index` and `second_model_index`; the rule's selector and `revised_selector` for `scope`, `first_scope` and `second_scope`; `match_by` `identity` (with `identity_scheme`), `placement` and `overlap` (with `minimum_overlap_ratio`) for the `identification_modes`; `compare_placement`, `compare_geometry` and `compare_coordinate_systems`; `properties` for `compared_properties`; `all_property_sets` or `property_sets` for `compare_properties`, `compare_quantities` (quantity sets are sets) and `compare_property_sets`. See [Model comparison](./comparison.md#the-comparison-as-a-rule). |
| `ComponentContainmentPlanSpec` (`ComponentContainment`) | `containment`: the inner components as the rule's selection, the outer ones as `counterparts`, `minimum_volume_ratio` for the classification, `combine_adjacent` for `combine_outer_components`, one `cover` row per dimension band (`faces` the host surface, `side` inside or outside, `minimum_metres`, `maximum_metres`), `minimum_count` and `maximum_count` for the counts per outer component, `report_orphans` for `forbid_orphans`. See [Containment and cover](./clash.md#containment-and-cover). |

`axioval-spec` itself is retired: no crate depended on it, and its
remaining plans and target list were shaped after particular host
applications. Rule packages are `axioval-ir`; writing them as another
format is an [export profile](./export.md). The crate name serves only a
retirement notice (`attic/axioval-spec-notice`).
