# Contributing

Read `AGENTS.md` and the nearest nested `AGENTS.md` before editing.

1. Add a failing test for behavior changes.
2. Keep core crates free of source-format and geometry-backend dependencies.
3. Update architecture and migration documentation with contract changes.
4. Run `./scripts/check.sh`.
5. Use scoped Conventional Commits on the linear `main` branch.

A capability is migrated only when the previous implementation is no longer the production owner and parity evidence is recorded.

Axioval is source-neutral and published publicly. Do not name a specific
commercial product, client, or private downstream consumer in tracked files or
commit messages; describe the capability, not its provenance. Interoperability
formats and public standards (IFC, IDS, `.cset`, OpenBimRL) may be named.

## Contributor licence terms

Axioval is licensed `AGPL-3.0-or-later OR LicenseRef-Commercial` (see
`LICENSE`). So that every release can keep both options, a contribution is
accepted only under these terms. By submitting a contribution (a pull request,
patch, or other material intended for inclusion), you agree that:

1. You keep the copyright in your contribution.
2. You grant rheonda, as the copyright holder of Axioval, a
   perpetual, worldwide, non-exclusive, royalty-free, irrevocable licence to
   use, reproduce, modify, distribute, and sublicense your contribution, and
   to license it, alone or as part of Axioval, under any terms, including the
   AGPL-3.0-or-later and proprietary commercial licences.
3. You grant the same holder, on the same terms, a licence under any patent
   claims you can license that your contribution, alone or combined with
   Axioval, would otherwise infringe.
4. The contribution is your original work, or you otherwise have the right to
   submit it under these terms. If an employer or client has rights in it,
   they have agreed to these terms.
5. Your contribution is provided "as is", without warranties of any kind.

A pull request states this agreement in its description: "I agree to the
contributor licence terms in CONTRIBUTING.md." A contribution without it is
not merged.
