# Axioval Engine repository instructions

Development is hot on `main`: use scoped Conventional Commits, keep history linear, and stage explicit paths only.

The public documentation system is mdBook plus workspace rustdoc, deployed by `.github/workflows/pages.yml`. Update the relevant page with every public contract or architecture change and keep `docs/src/SUMMARY.md` complete.

Run `./scripts/check.sh` before committing. The gate includes mutation-proven architecture boundaries, the 65-entry migration ledger, formatting, Clippy, tests, rustdoc, and mdBook.

The engine uses a source-neutral IR. IFC, STEP, ICDD, Axiolid, Solibri, and vendor CAD types are forbidden in `axioval-ir` and `axioval-engine`. OpenBIM and Axiolid adapters are independent peers and must not depend on each other.
