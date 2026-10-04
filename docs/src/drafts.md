# Rule drafts

An editor checks a rule while it is being built, without a model: the
draft is compiled within its ruleset context exactly as `check` compiles
it, and every refusal comes back as a diagnostic the editor can place.
The library entry points are `axioval_engine::draft::validate_rule` and
`validate_expression`; the command line is `axioval validate --rule`,
`--expression` and `--serve` ([Command line](cli.md#axioval-validate)).

## Diagnostics

| Field | What it holds |
| --- | --- |
| `code` | The refusal's kind: `syntax` or `shape` for a draft that does not parse, `unknownRule` for an expression placed in no rule, else the engine's (`invalidExpression`, `unknownDefinition`, `unknownParameter`, `missingParameter`, `invalidParameterType`, `invalidMeasured`, `invalidRuleId`, …). |
| `message` | The engine's message, as `validate` and `check` print it. |
| `rule`, `parameter` | The rule and parameter it is about. |
| `path` | The engine's path into an expression: `requirement.and[1].compare.right`. |
| `pointer` | A JSON pointer into the draft (for an expression draft, into the expression): `/parameters/requirement/value/operands/1/right`. |
| `line`, `column` | Where a draft that does not parse goes wrong. |
| `suggestion` | A fix: the nearest known definition, parameter, concept, measured value or rule for an unknown one (`did you mean `t.Cover`?`), or the parameter a rule must add. |

The compiler stops at the first refusal, so a draft has at most one
diagnostic; fixing it shows the next. A refusal elsewhere in the context
is reported too, with its rule and without a pointer.

Expression paths become pointers through the catalogue's field tables
([Authoring catalogue](catalogue.md)): node kinds alternate with their
fields, and an indexed kind (`and[1]`) names an item of its list of
operands.

## Dry runs

With a model, a valid draft is run as `check` runs it, and the
`ExpressionTraces` service makes the `expression` capability record how
it judged every object it selected, passes included: the verdict and every
subexpression it evaluated, with its value, in order. Other capabilities
report their findings and not-evaluated outcomes. A run without the
service records nothing and is unchanged.

## Editors

`--serve` keeps the context loaded and answers one JSON request per line,
so an editor can validate on every edit:

```json
{"rule": {"id": "cover", "definitionId": "…", "name": {"default": "Cover"}, "parameters": {…}}}
{"expression": {"kind": "compare", …}, "into": "cover", "parameter": "requirement"}
```

Each answer is one line: `{"valid", "rule", "diagnostics"}`, or
`{"valid": false, "error"}` for a request that is not JSON or names
neither a rule nor an expression.
