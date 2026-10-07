# Axioval Engine

Axioval Engine validates federated engineering data without making any source format or geometry kernel its internal model.

The runtime compiles normalized Axioval packages into trusted capability invocations. Source adapters provide semantic objects and evidence. Geometry adapters provide optional exact geometric evidence. Findings retain source identity and provenance.

Rules are composed from shared parts: values measured by trusted built-in code, expressions over intervals with three-valued truth, selectors and generic judges. The built-in capabilities are preconfigured compositions of the same parts (templates) behind a stable outside contract, so a package addresses each as one rule type, and an author can compose or fork the parts into a rule of their own. A value that cannot be decided is never a pass.

## Where to start

- **How it fits together:** [Architecture](./architecture.md) for the
  crates, the layers of a rule and how a run executes; the ADRs for why.
- **Writing rules:** [Capabilities](./capabilities.md) lists the built-in
  rule types and their parameters; [Composing rules](./composing-rules.md)
  walks through rules built from measured values and expressions, with
  [Expressions](./expressions.md) and
  [Derived properties and measured values](./derived.md) as the reference.
- **Building an editor:** the [authoring catalogue](./catalogue.md),
  [rule drafts](./drafts.md), [block editors](./block-editors.md) and the
  [JSON Schemas](./json-schema.md).
- **Running checks:** the [command line](./cli.md), and the
  [report sinks](./sinks.md), [review decisions](./decisions.md) and
  [IDS](./ids.md) chapters for what goes in and out.
- **Changing a capability:** [Capability templates](./templates.md),
  the [parity harness](./parity.md), [template performance](./performance.md)
  and [Contributing](./contributing.md).

## Why a dedicated IR?

IFC remains an important adapter, but its schema inheritance, STEP identity, file-centric lifetime and serialization history are not suitable as a universal runtime contract. A dedicated IR also admits proprietary CAD, database-backed digital twins, ICDD collections, and future IFCX-style layered compositions.

## Current status

The engine is being extracted from a production legacy compatibility implementation. The migration ledger records what is actually cut over; unchecked entries are not claims of support. 58 of the 70 built-in capabilities run as templates, each held to the implementation it replaced by the parity harness and the template benchmark.
