# Report sinks

A sink turns a finished `Report` into a format other tools read. Sinks depend
on `axioval-ir` alone: they see findings, not-evaluated outcomes and the
project the report was computed over, never the engine or a source adapter.

## BCF

`axioval-bcf` writes BCF 2.1 or 3.0 archives through `openbim-bcf`.

```rust,ignore
let export = axioval_bcf::export(
    &report,
    session.project(),
    &axioval_bcf::Options::new("axioval", "2026-09-26T10:00:00Z"),
)?;
std::fs::write("issues.bcfzip", export.to_bytes()?)?;
for object in &export.unanchored {
    eprintln!("no GlobalId, not selectable in a viewer: {object}");
}
```

| Report entry | Topic |
|---|---|
| Finding | `TopicType` is the severity (`Error`, `Warning`, `Info`); `Priority` follows it (`High`, `Normal`, `Low`); the labels are the rule id, then `Storey: <name>` and `Space: <name>` for a located finding |
| Not-evaluated outcome | `TopicType` is `Not evaluated`; no `Priority`, since no severity was decided; the description names the reason |

**Labels.** A topic's first label is its rule id. `Options::rule_labels`
adds labels per rule after it (before any location labels), and
`ruleset_labels` derives them from a
ruleset: `Folder: <path>` for a rule inside folders (folder names joined by
` / `, the root left out), then the rule's tags, trimmed, each once. Labels
never enter the GUID key. The CLI labels every check this way; with several
rulesets the keys are qualified by package as the rule ids are. A finding
with `categories` is labelled `Category: <outermost> / ... / <innermost>`
next, before its location's `Storey:` and `Space:` labels.

Every topic's description repeats the message, the rule, the source-qualified
object ids and each evidence locator with its exactness.

**Report tables are not exported.** A table of measured values (see
[Tables](./ir.md#tables)) is not an issue, so it writes no topic; a report
with tables writes the same archive as one without.

**Not-evaluated outcomes are exported by default.** An archive that lists only
findings reads as "everything else passed". `Options::include_not_evaluated`
turns them off explicitly.

**Components are selected by GlobalId**, read from the `ifc-globalid` alias the
IFC adapter attaches. The subject comes first, then related objects. An
object without the alias cannot be selected: its topic is still written, and
it is listed in `Export::unanchored`. A viewpoint never selects related
objects alone, because it would point the reviewer at the wrong element.

**Resource objects are never components.** A finding about a resource object
(an IFC material, a relationship; see `Report::resources`) keeps its topic and
its identity, keyed by the resource object's GlobalId where it has one and by
its source-qualified id otherwise, but no viewer shows it as an element: its
topic has no viewpoint, and it is listed in `Export::unanchored`. An id the
report neither finds in the project nor carries is refused (`UnknownObject`).

**Model-level topics.** A finding or outcome about a whole source or the
project (see [Scope](./ir.md#scope)) has no subject, so its topic has no
viewpoint and no component. Its description says `Source: <source>; no single
object` or `Project: no single object or source`, and lists the related
objects (for example the objects a count found) without selecting them.

**GUIDs are stable across re-exports.** A finding's topic GUID is its
[identity](./decisions.md#finding-identity) over the `ifc-globalid` scheme,
`Finding::id` when the host identified the report with that scheme. Topic
and viewpoint GUIDs are UUIDv5 over the rule, the objects' GlobalIds and the
message, so rechecking a revised
model reproduces the GUID of every issue still present, and BCF tools can
track it. Objects without a GlobalId fall back to their STEP-numbered identity
and cannot be tracked this way. When a federation holds two revisions that
produce the same issue, those topics are qualified by source to stay unique.
A source-level topic's GUID is keyed by its scope kind and related GlobalIds,
never by the source's name, so a revision saved under another file name keeps
it; it is qualified by source only when it would repeat. Object and rule-level
topics keep the GUIDs they had before scopes existed.

**Output is deterministic.** The caller supplies author and date, nothing reads
the clock, and identical input writes identical bytes. Written archives are
checked against the buildingSMART 2.1 schemas.

## Decisions

A finding carrying a reviewer's decision (see [Review decisions](./decisions.md))
writes it into its topic:

| Decision | Topic |
|---|---|
| `accepted` | `TopicStatus` `Accepted` (`STATUS_ACCEPTED`) |
| `rejected` | `TopicStatus` `Rejected` (`STATUS_REJECTED`) |
| `open` | the host's `Options::status`, as undecided |

Every decided topic gets one comment: the status, then `: ` and the
decision's comment when it has one (`Accepted: agreed with the engineer`),
by the decision's author at its date. When the finding changed since the
decision, the comment adds a line `Changed since the decision: severity
warning -> error` and the topic gets the label `Decision changed`
(`DECISION_CHANGED_LABEL`), so a reviewer can filter what to look at again.
The comment's GUID is UUIDv5 of `decision` in the topic GUID's namespace, so
a re-export reproduces it. Not-evaluated outcomes are never decided, and
stale decisions write no topic. A report without decisions writes the same
archive as before.

## Import

`axioval_bcf::import` reads a BCF 2.1 or 3.0 archive back, typically one the
sink wrote and a reviewer then worked through in another BCF tool, and maps
its topics onto [review decisions](./decisions.md) about a report's
findings. `import_topics` does the same for topics already read.

```rust,ignore
let imported = axioval_bcf::import(&std::fs::read("reviewed.bcfzip")?, &report, project)?;
report.apply_decisions(&imported.decisions)?;
for topic in &imported.unmatched {
    eprintln!("{:?} decided nothing: {:?}", topic.title, topic.reason);
}
```

**Matching.** A topic belongs to the finding whose identity over the
`ifc-globalid` scheme is its GUID, the identity the export wrote it under.

**What a matched topic decides.** It yields a decision only when it carries
review state; an untouched topic yields none, so exporting and importing
again decides nothing.

| Topic | Decision |
|---|---|
| `TopicStatus` `Accepted`, `Closed`, `Resolved` or `Done` (`ACCEPTED_STATUSES`, any case) | `accepted` |
| `TopicStatus` `Rejected` (`REJECTED_STATUSES`, any case) | `rejected` |
| any other status, with the export's decision comment or any other comment | `open` |
| author and date | of the latest of the export's decision comment and the topic's `ModifiedAuthor`/`ModifiedDate`; without either, of its last comment; without comments, of its creation |
| comment | the latest comment the export did not write; else the text of the export's decision comment, without its status word and change note |

The status comes from `TopicStatus`, never from comment text, so a status
changed in another tool wins. An imported decision has no basis, so whether
its finding changed since is `unknown`.

**Nothing is dropped.** Every topic that decides no current finding is listed
in `Import::unmatched` with its GUID, title, status and why: `NoFinding`
(the finding was fixed or changed into a new one, or another tool made the
topic), `NotEvaluated` (a not-evaluated outcome's topic, never decided),
`NoGuid`, or `Unreadable` (a matched topic whose dates carry no UTC offset,
or whose comment has no author). Two topics with one finding's GUID refuse
the import.

**Untrusted input.** An archive is read within `IMPORT_LIMITS` (32 MiB per
entry, 256 MiB in all), and every entry but an image is scanned before the
XML reader parses it: a tag with more than `MAX_ATTRIBUTES_PER_TAG` (64)
attributes refuses the archive, because the reader's duplicate-attribute
check is quadratic in their number (RUSTSEC-2026-0194).

## Cameras

A report carries no geometry, so the host passes what it measured:
`Options::bounds` maps objects to their axis-aligned `Bounds` in model
coordinates (metres). The CLI fills it with `--geometry`, from the proximity
service's extent of each object named by the report (the mesh box grown by
its chord deviation, so it encloses the true body).

```rust,ignore
let options = axioval_bcf::Options {
    bounds: Some(bounds), // BTreeMap<ObjectId, axioval_bcf::Bounds>
    ..axioval_bcf::Options::new("axioval", "2026-09-26T10:00:00Z")
};
```

A topic whose subject and related objects are all bounded gets two
viewpoints with the same selection: a perspective camera first, then an
orthogonal one. Both frame the union of the objects' bounds: they look at
its centre from above, south and east (direction `(-1, 1, -1)`, up
`(-1, 1, 2)`), from where a sphere around the union (its radius scaled by
`FRAME_MARGIN`, 1.2, and at least `MIN_FRAME_RADIUS_METRES`, 0.5 m) fits
the 60° field of view. The orthogonal camera stands at the same point and
shows the sphere's diameter. Coordinates are rounded to micrometres, so
output stays deterministic. The first viewpoint keeps the GUID it has
without a camera.

**A missing bound leaves the viewpoint without a camera**, never a guessed
one. With bounds supplied, such objects are listed in `Export::unframed`;
without any bounds (`None`, the default) viewpoints are written exactly as
before.

## Colouring

`Options::colors` colours every viewpoint: the finding's subject (or the
object a not-evaluated outcome names) in `Colors::subject`, the related
objects in `Colors::related`, so a reviewer tells the element that fails
from the ones it fails against. `Colors::default()` is opaque red
(`SUBJECT_COLOR`, `FFFF0000`) and opaque blue (`RELATED_COLOR`,
`FF0000FF`). A `Color` is ARGB, parsed from 6 (`RRGGBB`, opaque) or 8
(`AARRGGBB`) hex digits and always written as 8 uppercase digits, which
both BCF 2.1 and 3.0 accept. `None`, the default, writes no colouring, and
the archive is byte-identical to one written before colouring existed.

```rust,ignore
let options = axioval_bcf::Options {
    colors: Some(axioval_bcf::Colors {
        subject: "E53935".parse()?,
        ..axioval_bcf::Colors::default()
    }),
    ..axioval_bcf::Options::new("axioval", "2026-09-26T10:00:00Z")
};
```

## Visibility

`Options::isolate` hides everything but the objects a viewpoint selects:
it writes `DefaultVisibility="false"` with the subject and related objects
as exceptions, so a viewer shows the finding alone. Off by default: every
viewpoint shows the whole model (`DefaultVisibility="true"`, no
exceptions), as before. A topic without a viewpoint stays without one.

## Section box

`Options::section_box` cuts every viewpoint that has a fitted camera by six
clipping planes: a box around the union of the topic's objects' bounds,
grown by `SECTION_BOX_MARGIN_METRES` (0.5 m) on every side, so the walls
and slabs around a finding no longer hide it. Each plane passes through the
middle of its box face and points outwards (a plane clips what lies on the
side its direction points to), written low then high side of x, y and z,
rounded to micrometres. A viewpoint without a camera has no bounds to box
and is never clipped, never cut by a guessed box. Off by default: no
clipping planes, as before.

## BCF 3.0

`Options::version` selects `Version::V2_1` (default) or `Version::V3_0`.
BCF 3.0 requires a camera on every viewpoint, so a 3.0 export is refused
with `ExportError::MissingCamera`, naming an object without bounds, unless
every viewpoint has one. Topics without a viewpoint (model-level topics,
subjects without a GlobalId) need none. A 3.0 archive carries an
`extensions.xml` listing the types, statuses, priorities and labels its
topics use, and each camera an aspect ratio of 1.

## Not written

Snapshots are rendering and out of scope.
