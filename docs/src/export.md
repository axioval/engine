# Export profiles

Rule packages reach other formats through export profiles. A target format
can hold only part of what a package expresses, so every export returns the
artifact together with what it left out or reduced. `axioval-export`
(`crates/packages/export`) holds the framework every target shares; each
target implements it once. The IDS exporter is the first
([IDS export](./ids.md#export)).

## Profiles

An `ExportProfile` writes one format:

```rust,ignore
pub trait ExportProfile {
    fn id(&self) -> &str;
    fn format(&self) -> &str { self.id() }
    // Optional; defaults to none.
    fn expression_kinds(&self) -> &[&str] { &[] }
    fn export(&self, definitions: &[DefinitionPackage], ruleset: &RuleSetPackage)
        -> ExportOutcome;
    // Optional; each defaults to an unsupported outcome.
    fn export_report(&self, report: &Report) -> ExportOutcome;
    fn export_takeoff(&self, tables: &[ReportTable]) -> ExportOutcome;
    fn export_classification(&self, definitions: &[DefinitionPackage], ruleset: &RuleSetPackage)
        -> ExportOutcome;
}
```

- `id` is an open string (`ids`), never a closed enum: no crate lists the
  formats that exist, so a new target changes no other crate.
- `format` is the name a person reads (`IDS`), used in messages.
- `expression_kinds` declares the [expression](./expressions.md) node
  kinds the format can state, as packages write them (`and`, `compare`,
  `property`, ...); see [Expression rules](#expression-rules).
- `export` writes a ruleset. Report, takeoff and classification exports are
  optional; a profile that does not override one returns no artifact and a
  refused loss at `report`, `takeoff` or `classification`.

An `ExportOutcome` has four fields:

| Field | Meaning |
|---|---|
| `artifact` | The written bytes, `None` when nothing could be written |
| `exported` | The ids of the rules the artifact holds, in ruleset order |
| `losses` | Everything it does not hold as the package states it, in ruleset order |
| `contents` | What the artifact holds in the format's own terms (`3 specification(s)`), when it has such a unit |

Every rule of the ruleset is either in `exported` or has a refused loss
whose `path` is its id. An exported rule may still carry degraded losses.

## Losses

A `Loss` is `{ path, kind, reason }`: where in the package, which kind, and
why in words. There are two kinds.

- **Refused.** The item's checking meaning cannot be expressed in the
  format, so it is left out of the artifact.
- **Degraded.** Structure or presentation is reduced, and every verdict is
  unchanged: folders flattened into one level, hierarchy depth cut,
  languages dropped, a column the format cannot hold, annotations it has
  no place for.

This is a two-level fidelity scale on purpose. An item is exported in full,
exported with only its presentation reduced, or not exported at all. There
is no "partially supported" check: a check that runs with part of its
meaning would decide differently from the package while looking like it.

### The safety rule

**Anything that would change a check's result is Refused, never
Degraded.** That covers the capability, every parameter (a default made
explicit included), the applicability, severity, gates, grading and target
groups, and every concept by the names it binds to. A consumer may take a
degraded artifact as checking what the package checks; a refused rule is
listed so nobody believes it was checked. A profile that cannot tell
whether a reduction changes a verdict refuses.

`axioval-export` enforces this in its API test
(`crates/packages/export/tests/safety.rs`): a changed parameter,
applicability, severity or enablement is a difference, and the loss it
yields is refused; a changed name, description, message or explicit default
is no difference at all.

## Judging exactness the same way

Every profile judges exactness with the same comparator in
`axioval_export::compare`:

- `Catalog` collects the definitions and concepts of the definition
  packages.
- `Catalog::canonical` states what decides each rule as JSON: capability,
  parameters with the definition's defaults, applicability, enablement,
  severity, gate, auxiliary flag, severity bands and overrides, and
  categories. Concepts are replaced by the names they bind to and rule ids
  by their position; names, descriptions, messages and tags are left out.
  A `Comparison` adds what a format leaves out beyond that (a parameter
  that only words a finding) and whether object-type names ignore case.
- `substitute` does that replacement on any JSON value, and
  `first_difference` names the path of the first difference between two
  values (`rules[0].parameters.values`).
- `verdict_difference` compares two rule lists this way, and
  `difference_loss` turns a difference into its refused loss.

A profile that can read its own artifact back translates what it wrote into
a package again and compares it with the original: a rule that comes back
different is refused. `axioval_export::precheck` holds the checks most
formats need first: `pre_check` refuses, in this order, a disabled,
auxiliary, gated (itself or through a folder), graded, otherwise-severe or
group-targeted rule, and `is_gated`, `is_folder_gated`, `is_graded`,
`severity_name` and `selector` are there for a profile that states some of
those.

## Expression rules

An `expression` rule (`axioval:capability.expression`) computes its
requirement, and most formats can state few of its nodes. A profile
declares the node kinds it can state with `expression_kinds`; the default
is none, so a profile that says nothing refuses every expression rule.
`precheck::unsupported_expression_node(expression, "requirement",
profile.expression_kinds())` walks the tree, parents before operands and in
written order, and returns the first node of another kind as an
`ExpressionNode { path, kind }`, named by the path the engine names it
with in type errors and not-evaluated outcomes
(`requirement.and[2].compare.left`, see
[Expressions](./expressions.md#paths)); `expression_nodes` lists every
node with its path. The profile refuses the rule naming that node, for
example "expression node `requirement.and[1]` (`aggregate`) has no IDS
facet".

Declaring a kind only says the format may hold such a node: where it sits
and what it reads still decide, and the profile still proves every rule it
writes exact as any other, by reading it back. An expression the profile
cannot prove is refused, never degraded: a reduced expression decides
differently. A `deviation` grades findings and is refused like any
grading; a `message` only words a finding, which a profile leaves out of
the comparison as a presentation parameter.

### Templates and forked rules

A rule bound to a capability built as a [template](./templates.md) is
still a rule of that capability with its parameters: a profile writes it,
or refuses it, exactly as before the capability was rebuilt. A rule forked
from it is an `expression` rule like any other: exported only where the
profile's `expression_kinds` state every node of its requirement, and
otherwise refused naming the first node it cannot state.

## A host-owned format

A host application implements its own format's profile in its own crate,
outside this repository. It depends on `axioval-export` and `axioval-ir`
only, implements `ExportProfile` with an id of its choosing, and registers
the profile with its own frontend. Nothing in the engine names or
enumerates such formats, and none needs to change for one to be added.

A profile for a host's format typically:

1. walks the ruleset, running `pre_check` on each rule (and
   `unsupported_expression_node` on an expression rule's requirement) and
   refusing what the format cannot state;
2. writes each remaining rule, and records a degraded loss for structure it
   flattens or presentation it drops;
3. where it can read the format back, compares the read-back rules with
   `verdict_difference` and refuses every rule that differs.

## Command line

`axioval export --profile <id> --definitions … --ruleset … --out …` runs a
profile the CLI registers (see [Command line](./cli.md#axioval-export)).
Refused rules are listed on stderr as not exported, degraded items as
degraded; the command exits 0 when nothing was lost, 4 when the artifact
was written with losses, and 1 when nothing could be written.
`axioval ids export` is `export --profile ids`.
