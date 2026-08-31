# Capability model

A capability is trusted executable policy registered by a host application.

Each descriptor declares:

- stable capability ID and version;
- execution scope;
- accepted selector shapes;
- typed parameter signature and defaults;
- required semantic/evidence capabilities;
- result and exactness contract.

Compilation rejects unknown capabilities, duplicate registrations, missing/extra parameters, invalid values and unsatisfied static contracts.

Execution is also fallible. `Runtime::run` rejects a plan if its runtime registry no longer contains every compiled capability; registry drift can never turn a rule into an implicit pass.

At runtime, missing evidence does not become a pass. `CapabilityEvaluation` carries conclusive findings separately from object- or rule-level not-evaluated outcomes. The runtime binds every such outcome to the compiled `RuleId`, sorts it deterministically, and exposes it through `Report::not_evaluated()`. Reasons distinguish a missing service, backend outage, incomplete evidence, invalid evidence, and resource exhaustion.

A capability may return findings and not-evaluated outcomes together when only part of its selected universe was computable. Consumers must not interpret an empty findings list as a pass while `not_evaluated` is non-empty.

## Built-ins

`axioval-rules` contains reusable, vendor-neutral implementations. Vendor identity, proprietary format handling, localized Solibri text and oracle-only ordering remain adapters in `vendor/solibri`.

`axioval:capability.free-floor-circle` checks whether each selected spatial scope can contain an exact vertical cylinder. `diameter_metres` and `height_metres` are canonical-metre parameters. The request covers every other project object as a candidate obstacle and requires exact whole-base support on the selected scope with zero hidden gap. A complete exact no-placement proof emits `NO_FREE_FLOOR_SPACE_FOR_CIRCLE`; missing services, backend outages, or invalid/incomplete evidence emit not-evaluated outcomes instead.

## Adding a capability

1. Define or reuse canonical schema concepts and parameters.
2. Add failing contract and behavior tests.
3. Implement policy only; put source interpretation in an adapter.
4. Declare all evidence requirements.
5. Add deterministic and missing-evidence tests.
6. Register in the built-in registry.
7. Record Solibri parity and cutover in the migration ledger when applicable.
