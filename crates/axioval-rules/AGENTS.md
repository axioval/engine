# `axioval-rules`

Maintained source-neutral capability policy and shared rule algorithms.

Capabilities may depend on Axioval IR and typed host-service interfaces, never concrete OpenBIM, ICDD, Axiolid, Solibri, STEP, or vendor CAD types. Source interpretation belongs in adapters.

`property_rules.rs` owns property compliance policy; `selection.rs` owns fail-closed selector evaluation. Both must resolve property values and absences through `PropertyResolutionServiceHandle`. Never infer absence from a missing `Object.properties` entry, and never silently skip an object when a property selector cannot be resolved exactly.

`free_floor_circle.rs` and `free_floor_rectangle.rs` own exact grounded vertical-shape profiles. They must request whole-base support and all project objects as candidate obstacles. Missing services, backend failures, and invalid/incomplete proofs emit typed not-evaluated outcomes; they never produce a pass or compliance finding.
