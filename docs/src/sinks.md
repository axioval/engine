# Report sinks

A sink turns a finished `Report` into a format other tools read: BCF
issue archives, [spreadsheets](#spreadsheets), [HTML
reports](#html-reports). Sinks depend
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

Every decided topic gets the decision's own comment first: the status, then
`: ` and the thread's first comment when that is by the decision's author at
its date (`Accepted: agreed with the engineer`), by the decision's author at
its date. Every other comment of the thread follows, oldest first, by its
own author at its own date, under the GUID it came with
(`DecisionComment::id`) or else UUIDv5 over its author, date, text and
repetition in the topic GUID's namespace, so a re-export reproduces every
comment GUID and a BCF tool updates rather than duplicates them. A decision's `priority` replaces the
topic's `Priority`, and its `labels` follow the topic's own labels, each
once. Its `assigned_to` and `due_date` are the topic's `AssignedTo` and
`DueDate` (in 3.0 the derived `extensions.xml` lists the assignee among
its users), so both round-trip through an archive unchanged. When the
finding changed since the
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
let imported = axioval_bcf::import(
    &std::fs::read("reviewed.bcfzip")?,
    &report,
    project,
    &options.rule_labels, // as exported
)?;
report.apply_decisions(&imported.decisions)?;
for topic in &imported.unmatched {
    eprintln!("{:?} decided nothing: {:?}", topic.title, topic.reason);
}
```

**Matching.** A topic belongs to the finding whose identity over the
`ifc-globalid` scheme is its GUID, the identity the export wrote it under.

**What a matched topic decides.** It yields a decision only when it carries
review state (a status, a comment, an assignee, a due date, its own
priority or labels); an untouched topic yields none, so exporting and importing
again decides nothing.

| Topic | Decision |
|---|---|
| `TopicStatus` `Accepted`, `Closed`, `Resolved` or `Done` (`ACCEPTED_STATUSES`, any case) | `accepted` |
| `TopicStatus` `Rejected` (`REJECTED_STATUSES`, any case) | `rejected` |
| any other status, with the export's decision comment or any other comment | `open` |
| author and date | of the latest of the export's decision comment and the topic's `ModifiedAuthor`/`ModifiedDate`; without either, of its last comment; without comments, of its creation |
| comments | the text of the export's decision comment without its status word and change note, by its author at its date, then every other comment in archive order, each keeping its GUID as `id` unless it is the one the export derives; comments without text are skipped |
| assignee, due date | `AssignedTo`, `DueDate` (a date without UTC offset is unreadable) |
| priority | `Priority`, when it differs from the one the finding's severity gives |
| labels | every label the export does not write for the finding: not the rule id, not the `rule_labels` passed to `import` (pass those the archive was exported with), not `Decision changed`, and no `Folder: `, `Category: `, `Storey: ` or `Space: ` label |

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

**Untrusted input.** An archive comes from other parties, so it is read
within `IMPORT_LIMITS` (32 MiB per entry, 256 MiB in all, 100 000
entries), tighter than the reader's defaults; an archive beyond them is
refused (`ImportError::Read`).

## BCF API servers

`axioval-bcf-api` exchanges the same topics with a server speaking the
buildingSMART BCF API 3.0 (REST) instead of a file. It depends on
`axioval-bcf` for the mapping both ways, so a finding's topic has the same
GUID, its identity, as a file or over the API.

```rust,ignore
use axioval_bcf_api::{Auth, Client};

let client = Client::connect("https://bcf.example.com", &Auth::ClientCredentials {
    client_id: "checker".into(),
    client_secret: std::env::var("SECRET")?,
    token_url: None, // the server's /bcf/3.0/auth names it
})?;
let pushed = client.push("project-id", &report, project, &options)?;
let markups = client.pull("project-id")?;
let imported = axioval_bcf::import_topics(&markups, &report, project, &options.rule_labels)?;
```

**Signing in.** `Auth::Bearer` takes a token the host obtained,
`Auth::ClientCredentials` exchanges an OAuth2 client id and secret at the
server's `oauth2_token_url`, and `Auth::Device` runs the OAuth2 device flow
(RFC 8628): the host names the device authorization endpoint (BCF API 3.0
publishes none), shows the person the code, and the client polls until they
signed in. Credentials stay in `Auth` and `Client`; nothing writes them, and
their `Debug` output is redacted.

**Push.** `Client::push` exports the report with the given options and, per
topic, reads the server's topic by GUID: a new one is created (`POST`), an
existing one updated in place (`PUT`), never duplicated. For a finding the
report carries no decision about, the server's status, priority, assignee,
due date, stage and extra labels are kept: a push never overwrites review
state it did not decide. A decided finding's topic takes the decision's,
with its assignee and due date. Comments and viewpoints are added only when their GUID is
missing, so pushing twice adds nothing; viewpoints are immutable in the API,
and a camera without an aspect ratio gets `ASPECT_RATIO`. The server
attributes a pushed comment to the signed-in user.

**Pull.** `Client::pull` reads every topic of a project with its comments
as `openbim_bcf::Markup`s, so [import](#import) maps them exactly as it maps
an archive: a status changed on the server comes back as a decision,
unmatched topics are listed.

**Transport.** `ureq` without a TLS stack by default: an `https` server is
refused (`ApiError::TlsUnavailable`) rather than contacted in the clear.
The `native-tls` feature adds HTTPS through the platform's TLS library and
certificate store. The crate's tests run against an in-process server,
never a real one.

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

## Snapshots

A report carries no geometry, and the sink renders nothing. A host with
meshes passes a `SnapshotRenderer` to `export_with_snapshots`; every
viewpoint with a camera then asks it for a PNG, given a `SnapshotView`: the
camera, the subject and related objects, their colours (`Options::colors`,
or `Colors::default()` when the archive writes no colouring, so the image
always tells them apart), whether the view is isolated, and its clipping
planes. A viewpoint the renderer declines keeps no snapshot and its subject
is listed in `Export::unrendered`; a viewpoint without a camera has nothing
to render from and gets none. `export` never asks, so an archive without
snapshots is byte-identical to one written before they existed.

**A snapshot is illustrative, never evidence.** A topic with any snapshot
ends its description with `SNAPSHOT_NOTE` ("Snapshot: illustrative
rendering of tessellated bodies, not evidence."). Nothing is measured from
an image, and no outcome depends on one.

`axioval-bcf-snapshot` is the renderer, in its own optional crate so the
sink stays small. `Renderer::new` takes each object's triangle `Mesh` in
model coordinates (metres), and draws in pure Rust: a z-buffered software
rasteriser with flat shading, the subject and related objects in their
colours, every other object in `CONTEXT_COLOR` (light grey) unless the view
is isolated, triangles cut by the clipping planes before projection, and a
512-pixel-high image whose width follows the camera's aspect ratio
(`with_height` changes it). It declines when the subject has no mesh or the
camera is degenerate. It depends on meshes only, never on a source format
or a geometry kernel, and the same input gives the same bytes for a given
build.

```rust,ignore
let renderer = axioval_bcf_snapshot::Renderer::new(meshes); // BTreeMap<ObjectId, Mesh>
let export = axioval_bcf::export_with_snapshots(&report, project, &options, &renderer)?;
```

The CLI renders with `--bcf-snapshots` and `--geometry`, from the meshes it
compiled for the check.

## Not written

Header files, lines, bitmaps and view setup hints.

## Spreadsheets

`axioval-xlsx` writes a report as an Office Open XML workbook (`.xlsx`)
through `rust_xlsxwriter`, pure Rust.

```rust,ignore
let options = axioval_xlsx::Options {
    external_id_scheme: Some("ifc-globalid".to_owned()),
    ..axioval_xlsx::Options::new(1_790_416_800) // created, Unix seconds
};
std::fs::write("report.xlsx", axioval_xlsx::export(&report, project, &options)?)?;
```

**Sheets.** `Findings` (`FINDINGS_SHEET`) comes first: one row per finding,
then one per not-evaluated outcome, in report order, each stating its
`outcome` (`finding` or `not evaluated`), so the workbook never reads as
"everything else passed". Its columns are the finding id, rule, severity
or reason, scope, the object's kind and (with `external_id_scheme`) its
alias, categories, message, related objects, storeys, spaces, and the
decision with its author, date, assignee, due date, priority, labels and
whether the finding changed since. One sheet per [report
table](./ir.md#tables) follows, in report order: named for the table, or
for its rule and name when several tables share a name, made a valid and
unique sheet name (`sheet_names`).

**Table cells.** A table sheet's first row names the rule and the table,
the second is the header, frozen and filtered. The scope, the kind and
alias of its object and the group columns come first, then every column:

| Value | Text column | Number or quantity column: `<id> lower [unit]`, `<id> upper [unit]` | `<id> exactness` |
|---|---|---|---|
| exact | the text | the value twice, as numeric cells | `exact` |
| interval | | its two bounds, never collapsed into one number | `bounded` |
| unknown | a shaded blank | two shaded blanks | `not evaluated` |

The unit is the quantity's coherent SI unit or a number column's own unit
(`EUR`). An unknown text value is a shaded blank cell, never an empty
string, so a value stated as empty is told apart from one not evaluated.

**Deterministic.** The caller supplies the creation time; nothing reads
the clock and the archive's entries carry a fixed date, so identical input
writes identical bytes for a given build. Excel's limits are kept, not
worked around: a sheet over a million rows or a text over 32 767
characters refuses the export (`ExportError::Write`) rather than truncate.

## HTML reports

`axioval-html` renders a report as one self-contained HTML document from a
template.

```rust,ignore
let template = match custom {
    Some(text) => axioval_html::Template::parse(&text)?,
    None => axioval_html::Template::default(), // DEFAULT_TEMPLATE
};
let options = axioval_html::Options {
    external_id_scheme: Some("ifc-globalid".to_owned()),
    ..axioval_html::Options::new("2026-09-26T10:00:00Z")
};
std::fs::write("report.html", axioval_html::render(&report, project, &template, &options))?;
```

**Templates are data.** A template is HTML with placeholders and nothing
else: no conditions, loops, expressions or includes. Rendering copies the
text and replaces each placeholder with its slot's content, so a template
can reorder, wrap, restyle or leave out sections, but nothing in it runs,
and the sink executes no code from a template or a rule package.

| Placeholder | Content |
|---|---|
| `{{title}}`, `{{date}}` | `Options::title` and `Options::date`, escaped; may repeat |
| `{{style}}` | the built-in style sheet (`DEFAULT_STYLE`), for inside `<style>`; may repeat |
| `{{cover}}` | title, date, overall status (not passed, incomplete or passed, as the CLI's exit status) and the sources |
| `{{summary}}` | findings by severity, not-evaluated outcomes, rules, tables, decisions and stale decisions, counted |
| `{{rules}}` | per rule: status (from `Report::rules` when recorded), objects checked, findings, not-evaluated outcomes, tables |
| `{{categories}}` | findings counted per rule and category path |
| `{{findings}}` | per rule, every finding: severity, object (scope, kind, alias), categories and message, related objects, storeys and spaces, decision with its thread |
| `{{not-evaluated}}` | every not-evaluated outcome: rule, object, reason, message, location |
| `{{stale-decisions}}` | decisions naming no finding of this run |
| `{{tables}}` | every report table: units in the header, an exact value, an interval as both bounds (`2.999999 – 3.000001`), an unknown one as `not evaluated` |

Spaces inside the braces are allowed (`{{ summary }}`). `Template::parse`
refuses an unknown placeholder, a section placed twice, an unclosed `{{`,
and a template without `{{summary}}` or `{{not-evaluated}}`
(`TemplateError`), so no report hides that its run was incomplete.

**Self-contained and print-ready.** The style is inline, and nothing
outside the file is referenced: no script, stylesheet, font or image. Every
text from the report or project is escaped. The built-in style lays the
report out on A4 pages for print: the cover on its own page, each section
starting a page, table headers repeated and rows kept whole.

**Numbers are never rounded.** Each is written with its shortest exact
representation, so an interval's two bounds never print as one value.

**Deterministic.** The caller supplies the date; nothing reads the clock.
Findings follow report order within each rule, rules and categories are
sorted, so identical input renders identical bytes.

**PDF** is printing this HTML, outside the engine: a browser's print
dialog, or a headless browser (`chromium --headless --print-to-pdf=report.pdf
report.html`). The sink writes no PDF and depends on no browser.
