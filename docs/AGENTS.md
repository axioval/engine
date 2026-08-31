# Documentation

This directory is the mdBook source for the public GitHub Pages site.

## Boundaries

- `src/architecture.md` records dependency direction and public invariants.
- `src/ir.md` documents the source-neutral model independently of any adapter.
- `src/adapters.md` documents each adapter separately; never describe OpenBIM+Axiolid as one ownership layer.
- `src/capabilities.md` describes trusted executable capabilities and their evidence requirements.
- `src/migration.md` is the capability cutover ledger. Never claim migration without tests and Solibri consumption evidence.
- Generated rustdoc is nested at `/api` by CI; do not commit generated output.

Build with `mdbook build docs` and treat warnings/broken links as failures in CI.
