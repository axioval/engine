# `axioval-bcf`

BCF 2.1 and 3.0 issue archives from a `Report` and the `Project` it was computed over, and
review decisions read back from them (`src/import.rs`).

- Depends on `axioval-ir` and `openbim-bcf` only. Never the engine, never a
  source adapter, not even as a dev-dependency: a dev-dependency on an
  unpublished sibling breaks workspace package verification. The facade's
  `tests/ifc_bcf.rs` pins `IFC_GLOBAL_ID_SCHEME` to the adapter's
  `IFC_GLOBAL_ID` and runs IFC import through export.
- Every finding and, unless the host opts out, every not-evaluated outcome is a
  topic. Dropping not-evaluated outcomes by default would make an incomplete
  check read as a pass.
- A viewpoint selects the subject first. Never write a viewpoint of related
  objects alone: it would highlight the wrong element. An object without a
  GlobalId alias keeps its topic and is listed in `Export::unanchored`.
- A source- or project-scoped entry has no subject, so its topic has no
  viewpoint and no component, and nothing in it is `unanchored`.
- A resource object the report carries (`Report::resources`) is never a
  component, even with a GlobalId: its topic has no viewpoint and it is
  listed in `Export::unanchored`. Resolve ids through `Report::object`; an
  id neither the project nor the report holds is `UnknownObject`.
- GUIDs are UUIDv5 over rule, GlobalIds and message, so they survive
  re-export. The key is derived by `axioval_ir::finding_ids` and
  `not_evaluated_ids` over `IFC_GLOBAL_ID_SCHEME`, the same identity a host
  records decisions against; never build a key here. A scoped finding's key
  marks its scope kind but never names the source, which would change with
  the file name; `tests/export.rs` pins the object and rule-level GUIDs.
  Changing `axioval_ir::identity::NAMESPACE` or the key layout changes every
  GUID and decision key a user has ever received; treat both as a
  compatibility contract.
- A finding's `decision` sets the topic status (`Accepted`, `Rejected`; an
  open decision keeps `Options::status`) and adds its own comment by the
  decision's author at its date, GUID `v5(topic, "decision")`, carrying the
  thread's first comment when that is the decider's own; a changed one
  adds the `Decision changed` label. The rest of the thread follows under
  `DecisionComment::id` or `ThreadGuids` (v5 over author, date, text and
  repetition); import derives the same GUIDs to tell them from a tracker's,
  so keep export and import on `ThreadGuids`. Not-evaluated outcomes are never
  decided. Without decisions the archive is byte-identical.
- A finding's `Priority` follows its severity, unless its decision sets
  one; a not-evaluated outcome has none, because no severity was decided.
  Never invent one for it. A decision's labels follow the topic's own.
- A decision's `assigned_to` and `due_date` are written as the topic's
  `AssignedTo` and `DueDate`, never into comment text. Import reads them from
  `AssignedTo`/`DueDate`, a priority only when it differs from the
  severity's, and only labels the export does not write
  (`exported_labels` plus the `Folder:`/`Category:`/`Storey:`/`Space:`
  families).
- Labels are the rule id first, then the host's `rule_labels` for that
  rule (`ruleset_labels`: folder path, tags), then `Category: a / b` from
  `Finding::categories`, then the location's, each once. Labels never enter the GUID key: relabelling a rule must not change
  its topics' GUIDs.
- Never read the clock or invent GUIDs; the caller supplies author and date.
- Cameras come only from host-supplied `Options::bounds`. A missing bound
  leaves the viewpoint without a camera (listed in `Export::unframed`),
  never a guessed one. The perspective viewpoint keeps the GUID of the
  camera-less viewpoint; the orthogonal one is derived beside it. Without
  bounds the archive is byte-identical to one written before cameras.
- 3.0 requires a camera on every viewpoint: refuse the export with
  `MissingCamera` rather than drop a viewpoint or write 2.1 instead.
- `Options::colors` colours the subject (`selection[0]`) and the related
  objects apart in every viewpoint; `None` (the default) writes no
  colouring and the archive is byte-identical. `Color` always writes 8
  uppercase hex digits: BCF 2.1's schema refuses lowercase.
- `Options::isolate` writes `DefaultVisibility` false with the whole
  selection as exceptions; off (the default) keeps today's everything-visible
  output. Never isolate by default: a viewer would hide context the host
  did not ask to hide.
- `Options::section_box` clips only viewpoints with a fitted camera, by a
  box around the same union of bounds the camera frames; a viewpoint
  without bounds is never clipped by a guessed box. Planes point outwards.
- Snapshots come only from a host's `SnapshotRenderer` through
  `export_with_snapshots`; this crate never renders and never depends on a
  renderer. `export` never asks for one, so its output stays
  byte-identical. A topic with any snapshot ends its description with
  `SNAPSHOT_NOTE`: snapshots are illustrative, never evidence. A declined
  viewpoint keeps none and its subject goes to `Export::unrendered`.
- Colouring, visibility and clipping planes are written through
  `openbim-bcf`'s `Viewpoint` fields only; never post-process the writer's
  XML.
- Run `cargo test -p axioval-bcf` and `cargo test -p axioval --all-features`;
  `tests/export.rs` is the contract and reads every archive back.
- A located entry's topic is labelled `Storey: <name>` and `Space: <name>`
  after the rule id (the place's id when unnamed). Labels never enter the
  GUID key: locating a finding must not change its GUID.
- `import` reads archives within `IMPORT_LIMITS` through `openbim-bcf`'s
  bounded reader; never read untrusted bytes without limits. Never add an
  advisory ignore to `deny.toml` for the reader: wait for the upstream fix.
- Import matches topics by GUID to `finding_ids` over `IFC_GLOBAL_ID_SCHEME`.
  The status comes from `TopicStatus` only (`ACCEPTED_STATUSES`,
  `REJECTED_STATUSES`, anything else open), never from comment text. An
  untouched topic decides nothing, so export then import is a no-op. Every
  topic that decides no current finding goes to `Import::unmatched` with a
  reason; never drop one. A date without a UTC offset is `Unreadable`,
  never guessed as UTC. The export's decision comment (GUID
  `v5(topic, "decision")`) is recognised by GUID, and its status word and
  change note are stripped from the text.
