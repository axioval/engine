# Command line

The `axioval` binary (`axioval-cli`) is the host that composes the pieces: the
IFC source adapter reads the model, the engine runs the packages, and the BCF
sink writes issues. Each piece is also usable on its own as a library.

## `axioval validate`

```bash
axioval validate --definitions definitions.json --ruleset ruleset.json
```

Binds a ruleset to its definition packages and the built-in capabilities
without a model. Exits 0 when the ruleset compiles, 1 otherwise.

## `axioval check`

```bash
axioval check --model building.ifc \
  --definitions definitions.json --ruleset ruleset.json \
  [--report result.json] [--bcf issues.bcfzip] \
  [--bcf-author NAME] [--bcf-date 2026-09-26T10:00:00Z]
```

Runs the ruleset over an IFC2X3 or IFC4 STEP model. The source document is
named by the model's file name, so a result does not depend on the directory
it was checked from.

### Output

The JSON result goes to stdout, or to `--report`:

```json
{
  "report": { "findings": [], "not_evaluated": [] },
  "integrity": [
    { "code": "identity.invalid-global-id", "severity": "warning",
      "message": "...", "locator": "ifc:sha256:...:global-id-invalid:#2" }
  ]
}
```

`report` is the engine's `Report`. `integrity` lists irregularities of the model
itself (see [Independent adapters](./adapters.md)); they are not rule findings.
Integrity issues and a one-line summary also go to stderr.

`--bcf` also writes a BCF 2.1 archive (see [Report sinks](./sinks.md)). The
topic date is `--bcf-date`, else `SOURCE_DATE_EPOCH` when set, else the current
time, all in UTC. With `SOURCE_DATE_EPOCH` set, the same inputs write
byte-identical archives. Objects the archive cannot select, because they have
no valid unique GlobalId, are named on stderr.

Everything is built before anything is written: a failing run leaves no
partial report or archive behind.

### Exit status

| Status | Meaning |
|---|---|
| 0 | Every rule was evaluated and nothing was found |
| 3 | At least one finding |
| 4 | No finding, but part of the check was not evaluated: the model has **not** passed |
| 1 | The check could not run: unreadable input, a package that does not compile, a model that does not import, or output the BCF writer refuses |
| 2 | Invalid command-line usage |

A finding takes precedence over incompleteness: status 3 can still come with
not-evaluated outcomes in the report. Automation that only needs pass or fail
treats any non-zero status as a failure.

### What runs today

Only semantic evidence is wired: properties, relationships, classifications
and the type hierarchy. No geometry backend is attached, so rules that need
geometry report not evaluated (status 4), never pass.
