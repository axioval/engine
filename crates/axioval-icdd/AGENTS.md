# `axioval-icdd`

ICDD document/link project assembly contracts and in-memory conformance double.

- Assembly validates manifest structure only; it must not introduce federation, semantic identity, or checking policy.
- Preserve manifest ordering and fail closed for duplicate or missing document references.
- `UnavailableIcddContainerReader` is intentional until an ISO 21597-1 reader is integrated behind `IcddContainerReader`.
- Run `cargo test -p axioval-icdd`; `tests/conformance.rs` is the assembly contract.
