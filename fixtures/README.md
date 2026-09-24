# Contract fixtures

`schema-v0.1.0/` is copied byte-for-byte from the checked normalized output of
the Axioval MCS `examples/minimal` package (`examples/minimal/expected/*.json`)
at commit `307ce0805b7ae782d0c39a98b6442a4e6b9325c8`.

These files are executable compatibility fixtures: the Rust binder must
deserialize and bind them exactly, reject missing or mismatched definitions and
parameters, and execute `axioval:capability.property-exists` without evaluating
package-provided code. Because they are real MCS output rather than hand-written
JSON, a contract drift between MCS and the engine fails here first.

Refresh them by copying the same two files again and updating the commit above;
never edit them by hand.
