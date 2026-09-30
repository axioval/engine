# Review decisions

A reviewer goes through a model's findings and decides each one: this clash
is accepted, that missing property is a false positive and rejected. When the
revised model is checked again, those decisions must still be there, on the
findings that are still there, or every reviewed finding reappears as open.

Decisions are keyed by a finding's **stable identity**, carried over by a
re-check to the finding with the same identity, and never hide anything.

## Finding identity

`Finding::id` is a `FindingId`, a UUIDv5 over:

- the rule id;
- the finding's objects, subject first, then related objects, each by its
  alias in a **stable scheme** the host names (an `ExternalId` scheme whose
  values survive a re-export), or its source-qualified id without one;
- for a finding about a source or the project, the scope kind (`source` or
  `project`), never the source's name, which changes with the file name;
- the message.

When the same key occurs twice in one report (two revisions of one model
federated in one project), those findings, and only those, are qualified by
the sources they touch, so every identity in a report is distinct.

```rust,ignore
report.identify_findings(session.project(), "ifc-globalid")?;
```

`Report::identify_findings` sets every finding's id; `axioval_ir::finding_ids`
returns them without changing the report. The runtime never sets them: which
scheme is stable is the host's knowledge, not the engine's. A report without
ids serializes exactly as before, `id` absent.

The identity is the BCF sink's topic GUID: the sink derives it with the same
function over the `ifc-globalid` scheme, so a decision recorded against a
finding and the BCF topic a reviewer saw it as are one key. The key layout
and the namespace (`axioval_ir::identity::NAMESPACE`) are a compatibility
contract.

What is and is not part of the identity:

| Change between revisions | Identity |
|---|---|
| objects renumbered, stable aliases kept | kept |
| model saved under another file name | kept |
| location, severity, evidence, categories' data | kept |
| a measured value written into the message | new: the old decision becomes stale |
| an object without a stable alias renumbered | new |

## Decisions

A `Decision` names the finding it is about and records:

| Field | Meaning |
|---|---|
| `finding` | the finding's identity |
| `status` | `accepted`, `rejected` or `open` |
| `author` | who decided; never blank |
| `date` | when, an ISO 8601 date-time with offset |
| `comment` | why: one comment by the decider at the decision's date; optional |
| `comments` | the thread instead, oldest first: each with `author`, `date`, `text`, and the `id` an issue tracker gave it (a BCF comment GUID); optional |
| `assigned_to` | who deals with the finding; optional, never blank |
| `due_date` | by when, an ISO 8601 date-time with offset; optional |
| `priority` | how urgent, in the project's own vocabulary (`High`, `Critical`); optional, never blank |
| `labels` | the reviewer's labels, in order; optional, none blank |
| `basis` | the finding as decided: rule, message, severity, evidence count and inexact evidence count; optional |

A decision holds an ordered thread of comments (`Decision::comments`, each a
`DecisionComment`): several reviewers comment over time. On the wire, a
thread of exactly one comment by the decision's author at its date and
without an `id` is written as the single `comment` it always was, so files
and reports written before threads read and serialize byte for byte as
before; any other thread is `comments`. A decision with both is refused, as
is a comment without an author.

`Decisions` holds at most one per finding, ordered by identity. Its file
form is strict: unknown fields, an unknown status, a blank author, a date
without offset and two decisions about one finding are all refused.

```json
{
  "decisions": [
    {
      "finding": "5c1f0c9e-6a0b-5d53-9a8e-2f3b8f6c1d20",
      "status": "rejected",
      "author": "A. Reviewer",
      "date": "2026-09-27T08:00:00Z",
      "comment": "the wall is a lining; contact is not required",
      "basis": {"rule_id": "slab-contact", "message": "...", "severity": "error",
                "evidence": 3, "inexact_evidence": 0}
    }
  ]
}
```

## Carrying decisions over

`Report::apply_decisions(&decisions)` marks every finding whose identity a
decision names with a `FindingDecision`: its status, author, date and
comment, its assignee, due date, priority and labels, and whether the
finding changed since. A decision or report without assignee, due date,
priority or labels serializes byte for byte as before they existed. Decisions naming no finding
are **stale** and listed in `Report::stale_decisions`, by identity: the
finding was fixed, or changed enough to be a new finding. Applying again
replaces what was applied before.

- A decision only marks. It never removes a finding, never changes its
  severity, and a report's findings, not-evaluated outcomes and tables are
  the same with and without decisions.
- Not-evaluated outcomes are never decided: an outcome that could not be
  evaluated is not a finding anybody reviewed.
- Decisions need identities: applying any to a report with an unidentified
  finding fails (`DecisionError::Unidentified`) rather than calling every
  decision stale.

## Changed evidence

A finding with the same identity may still differ from what was decided: a
revised model graded into another severity band, a new override holding,
more or fewer facts deciding it, facts turning approximate. Where the
decision recorded its basis, the carried decision compares it with the
finding now:

| `evidence` | Meaning |
|---|---|
| `unchanged` | rule, message, severity and both evidence counts match |
| `changed` | at least one differs; `changes` lists each facet as decided and now |
| `unknown` | the decision has no basis, so nothing could be compared |

```json
"decision": {
  "status": "accepted", "author": "A. Reviewer", "date": "2026-09-27T08:00:00Z",
  "evidence": "changed",
  "changes": [{"facet": "severity", "decided": "warning", "now": "error"}]
}
```

Evidence **locators** are deliberately not compared. A source renumbers its
entities on every export, so the locators of an unchanged model change
anyway; comparing them would flag every decision after every re-export and
teach reviewers to ignore the flag. A changed measured value is not missed:
it is part of the message, so the finding gets a new identity and its old
decision becomes stale. Without a basis the answer is `unknown`, never a
claimed `unchanged`.

## Command line

`axioval check` identifies every finding over GlobalIds, and `--decisions
FILE` applies a decisions file; `axioval decide` records decisions about a
saved result's findings, with their basis. See
[`axioval decide`](./cli.md#axioval-decide).

## BCF

A decided finding's topic carries the decision (see
[Report sinks](./sinks.md#decisions)): `TopicStatus` `Accepted` or
`Rejected` (an open decision keeps the host's status), one comment by the
decision's author at its date, followed by the rest of its comment thread,
and the label `Decision changed` when its evidence changed. A decision's priority replaces the one the severity
gives, its labels follow the topic's own, and its assignee and due date
are the topic's `AssignedTo` and `DueDate`. Stale decisions have no
topic.

The way back is [`axioval_bcf::import`](./sinks.md#import): a topic reviewed
in another BCF tool becomes a decision about the finding whose identity is
its GUID. A closed, resolved, done or accepted topic accepts it, a rejected
one rejects it, and a topic with comments but another status is `open`;
every comment is carried, in order, as the decision's thread. `AssignedTo`, `DueDate`, a priority other than the
severity's and labels the export does not write are read back as the
decision's. Topics that decide no current finding are listed
as unmatched, like stale decisions, and never dropped. `axioval check
--decisions-from reviewed.bcfzip` applies them in place of a decisions file.

A BCF API 3.0 server is the same way round: `axioval bcf push` creates or
updates a result's topics on it, and `axioval bcf pull` reads its review
state into a decisions file, each decision with its finding's basis (see
[BCF API servers](./sinks.md#bcf-api-servers)).
