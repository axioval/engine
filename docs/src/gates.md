# Gated rules

Capabilities never feed one another. A rule instance may still depend on how
another rule of its ruleset fared: check fire-door hardware only on the doors
that failed the door-type rule, or run a detailed rule only if a summary rule
failed. The runtime runs the other rule first and reads its refined outcomes.

## Gates

A rule instance, or a rule folder, declares a `gate`: the other rule and one
of four conditions.

```json
"gate": {"rule": "door-type", "condition": "failedObjects"}
```

| Condition | The gated rule runs |
|---|---|
| `allIfPassed` | on its whole selection, if the other rule passed |
| `allIfFailed` | on its whole selection, if the other rule failed |
| `passedObjects` | on the objects the other rule passed only |
| `failedObjects` | on the objects the other rule failed only |

A folder's gate applies to every rule in the folder and its subfolders,
together with each rule's own gate. A gate must name a rule outside the
folder that declares it.

### Whole-rule conditions

The other rule *passed* with no finding and nothing left not evaluated, and
*failed* with any finding, at any scope. A gate that does not hold skips the
rule: it reports nothing, and its rule status is `skipped`. When the other
rule reported no finding but left something not evaluated, whether it passed
cannot be decided: the gated rule reports one not-evaluated outcome about the
whole rule (`incomplete_evidence`), never a pass and never a skip. A rule
gated on a skipped rule is skipped too.

### Object conditions

An object condition narrows the gated rule's applicability selector: the
compiled selector is `allOf` the gate's `ruleOutcome` selector and the rule's
own. How the other rule judged each object:

- An object that is the subject of one of its findings *failed*.
- An object it left not evaluated is *undecided*.
- An object its applicability selector surely did not select is *not
  selected*, and matches neither condition.
- A selected object is *undecided* when the other rule reported about its
  source or the whole model as a whole, and when its selection could not be
  decided; otherwise it *passed*.
- A skipped rule passed and failed no object.

An undecided object is not evaluated by the gated rule
(`incomplete_evidence`), never checked and never left out. The other rule's
selection is its applicability selector, the one rule summaries count.

## `ruleOutcome` selectors

The object conditions are selectors any selector may use: in an
applicability, a target group, a selector parameter or a table cell, and a
severity override.

```json
{"kind": "ruleOutcome", "rule": "door-type", "outcome": "failed"}
```

`outcome` is `passed` or `failed`, judged as above.

## Order and compilation

The plan runs every rule after the rules its gates and `ruleOutcome`
selectors read, and otherwise by rule ID (`ExecutionPlan::rules`). A reference
is resolved within its ruleset; compiled with others
(`compile_rulesets`), references are qualified by package like the rule IDs.
Compilation refuses (`EngineError::InvalidDependency`):

- a reference to a rule the ruleset does not define;
- a rule depending on its own outcome, or rules depending on one another in
  a cycle;
- a rule reading another rule per object when the host registered no outcome
  refiner, which records that rule's selection
  (`OutcomeRefiner::evaluate_selector`).

A rule reading a disabled rule, or one that cannot run as authored, is
compiled not executable and reported not evaluated (`invalid_declaration`).
