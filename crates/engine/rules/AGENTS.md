# `axioval-rules`

Maintained source-neutral capability policy and shared rule algorithms.

Capabilities may depend on Axioval IR and typed host-service interfaces, never concrete OpenBIM, ICDD, Axiolid, STEP, or vendor CAD types. Source interpretation belongs in adapters.

`property_rules.rs` owns property compliance policy; `selection.rs` owns fail-closed selector evaluation. Both must resolve property values and absences through `PropertyResolutionServiceHandle`. Never infer absence from a missing `Object.properties` entry, and never silently skip an object when a property selector cannot be resolved exactly. `property-exists` means exact presence even when a source value is null or blank; `property-required` is the stronger non-empty contract and treats exact absence, null, and blank text as violations without converting adapter failures into findings. `property-data-type` adds the source-declared type to that contract; an unreported type is not evaluated, never a match. `property_value.rs` owns `property-value`: lexical constraints cast to the resolved value's kind. A literal or constraint that does not fit the value is not evaluated, never a pass or a violation. `attribute_value.rs` applies the same constraints to direct attributes through `AttributeServiceHandle`. `xsd_pattern.rs` translates XML Schema patterns; refuse any construct it cannot map exactly rather than approximate it.

`free_floor_circle.rs` and `free_floor_rectangle.rs` own exact grounded vertical-shape profiles. They must request whole-base support and all project objects as candidate obstacles. Missing services, backend failures, and invalid/incomplete proofs emit typed not-evaluated outcomes; they never produce a pass or compliance finding.

`clash.rs` and `distance.rs` own pairwise policy over `ProximityServiceHandle`; `pairs.rs` holds their shared selection and broad-phase set-up. An object whose extent cannot be read is reported not evaluated, never skipped. Meeting surfaces with no penetration measurement are not evaluated, not passed. Distance maxima may conclude "none within" only when every counterpart was measured.

`comparison.rs` owns the semantic session diff. Match by external identity scheme only, never by `ObjectId`. Report unidentified objects, ambiguous identities and unreadable facets; never compare an unknown as empty.
