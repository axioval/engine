# Contract fixtures

`schema-v0.1.0/` is copied from the normalized output of the Axioval Schema minimal example at commit `34105f3b82f114928921318f81d4b5390bc8ec31`.

These files are executable compatibility fixtures: the Rust binder must deserialize and bind them exactly, reject missing or mismatched definitions and parameters, and execute `axioval:capability.property-exists` without evaluating package-provided code.
