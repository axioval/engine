# `axioval-axiolid`

Source-independent geometry-evidence contracts and in-memory conformance double.

- This crate must remain independent of `axioval-openbim`; proprietary CAD sources are first-class inputs.
- Preserve exactness and provenance; never upgrade approximate evidence or emit policy findings.
- `UnavailableGeometryBackend` is intentional until a kernel/SDK implementation is integrated behind `GeometryBackend`.
- Run `cargo test -p axioval-axiolid`; `tests/conformance.rs` protects source scoping.
