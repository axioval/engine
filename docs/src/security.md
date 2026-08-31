# Security

See the [security policy](https://github.com/axioval/engine/blob/main/SECURITY.md) for private reporting.

The primary application-security invariant is that rule packages are untrusted data. They cannot execute code, select arbitrary filesystem paths, issue network requests, or load native libraries. Only capabilities registered by the host application may run.

Adapters must bound input sizes, recursion, graph traversal, decompression, geometry work and report volume. Resource exhaustion is an explicit run outcome rather than a process crash or silent partial pass.

Dependency policy is enforced by `cargo-deny`: known advisories, unapproved licenses, wildcard requirements, unknown registries and unknown Git sources fail the gate. GitHub Actions use immutable commit SHAs, and downloaded documentation tooling is checksum-verified before execution.
