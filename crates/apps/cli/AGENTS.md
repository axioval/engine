# `axioval-cli`

Strict command-line entry point for normalized Axioval packages and validation runs.

CLI output and exit codes are public automation contracts. Parse packages fail closed, write diagnostics to stderr, and use non-zero status for invalid input or execution failure.

- `--ruleset` repeats for `check` and `validate`, compiled through `compile_rulesets`; qualified rule ids (`package-id/rule-id`) are a report contract.
- `check` exit status: 0 complete and clean, 3 findings, 4 not evaluated without findings, 1 failure, 2 usage (clap). Status 4 exists so an incomplete check can never exit 0; never fold it into 0.
- Build every output before writing any, so a failing run leaves no partial report or archive.
- The CLI is the host: it may enable several facade features and compose an adapter with a sink. Libraries never do.
- Never read the clock for BCF output when `--bcf-date` or `SOURCE_DATE_EPOCH` is given.
- `src/digest.rs` owns the saved result format (`CheckOutput`) and its two views. The summary must stay bounded by distinct groups, never by entry count; the listing must page. Every "next" hint must run unchanged in a POSIX shell: quote through `shell_quote`, which quotes `#` because an unquoted `#` starts a comment.
- Report tables are the `tables` section: one summary group per table (rows counted, columns as the message, a grouped table's group columns first), one listing entry per row (a grouped row prefixed by its group values in brackets). They never affect the status. `report --csv` (`digest::table_csv`) prints exactly one table, selected by `--rule` and `--table`; its header (`scope`, group columns, `<id>_lower`/`<id>_upper` per numeric column) is a public contract, and an unknown value is empty, never zero.
- The result's `objects` field is additive (`serde(default)`), so results saved before it existed still load. It labels resource objects from `Report::resources` (`Report::object`), never from the project alone.
- `--geometry` is the host layer's IFC→Axiolid bridge, `axioval-host-ifc-axiolid` (`crates/hosts/ifc-axiolid`), taken through the facade's `ifc-geometry` feature as `geometry`. It belongs there and nowhere else: the adapters must not depend on each other, and the CLI must not fork it. Read and follow that crate's `AGENTS.md` before changing anything about meshing, the geometry services, the local frame or alignments. The CLI keeps only command-line concerns: `src/bounds.rs` (BCF cameras), `src/timings.rs` (`--timings`), `snapshot_meshes` (kept meshes for BCF snapshots), printing the `geometry` record and `compare`. `tests/check.rs` stays the bridge's end-to-end test, and its unit tests moved with it.
- Every model is read through `import` in `main.rs` (`compare` too): an ifcZIP archive (zip magic or `.ifczip`) through the adapter's `read_ifc_zip`, named `archive/member`, and the member's STEP bytes are what geometry meshes. Never unzip here. An ifcXML document (`is_ifc_xml`) is imported with `import_ifc_xml_session`, and the bridge parses its bytes with the adapter's `read_ifc_xml`, never the codec directly, so geometry sees the same proven model; STEP bytes likewise go through the adapter's `read_ifc_step` (the session's reader, which accepts a REAL without its decimal point and reports it), never `StepCodec`. `ifc-step` is a dev-dependency only.
- `--model PATH[:DISCIPLINE]` repeats: each file is imported as its own session and `EvidenceSession::federate` combines them before geometry is attached. The bridge meshes every source into one set only where it shares the first source's frame (see its `AGENTS.md`); `--declare-shared-frame` (with `--geometry`) sets `Options::with_declared_shared_frame`, and every source measured on it is named in the additive `geometry.declared_frame_sources`, noted on stderr and counted in the summary (`GeometryCounts::declared_frame`), never shown as read. Never key anything on a bare local id: two files share `#10`. Each model's file name is stated as its source's `fileName` metadata. `--discipline-map FIELD:PATTERN=DISCIPLINE` is applied after federation; the result's additive `sources` field lists each source's discipline and origin (`declared`, `mapped` with rule and value) or why the map left it without one.
- `src/compare.rs` is `axioval compare`: two single-model sessions matched by `GlobalId` through `axioval_rules::compare_sessions` (`--property-set` and `--all-property-sets` list sets through enumeration; relationship kinds always, `with_relationships`). The same comparison as a rule is the `model-comparison` capability over `check --model a.ifc:base --model b.ifc:revised`; `tests/check.rs` runs it. Its result is a `CheckOutput` with the additive `comparison` field (owned by `digest.rs`), so `report`, `--summary` and `--bcf` stay shared through `emit`; exit status as for `check`. Two files with one name become `name@base` and `name@revised`; never let the two revisions share a source id. `--geometry-mode mesh` (requires `--geometry`) compares surfaces across the two sessions' own proximity services (`with_mesh_geometry`); its records carry an additive `witness` per change and the comparison's `geometry_mode`, both omitted otherwise so bounds results stay byte-identical; a pair with exact boundaries on both sides is measured between them (measure `boundary`). `tests/compare.rs` runs it end to end, round columns between boundaries included.
- Walkability takes every role from its request; metric routing takes surfaces (`IfcSpace`), portals (`IfcDoor`, opening voids) and connectors (stairs, ramps, transport elements) from IFC classes. Never declare a door clear width from `OverallWidth`: it includes the lining, and an overstated clear width turns an undecided route into a false pass.
- BCF cameras come from `bounds::bounds` (`src/bounds.rs`): the proximity service's enclosing extent of each object the report names, only with `--geometry`. An unmeasured object is left out, never estimated; without `--geometry` pass `None` so the archive stays byte-identical. `--bcf-version 3.0` fails (status 1, nothing written) when a viewpoint has no camera.
- BCF colouring is on with `--geometry` or when `--bcf-subject-color`/`--bcf-related-color` is given, off with `--bcf-no-color`; without any of them and without `--geometry` pass `colors: None` so the archive stays byte-identical.
- `--bcf-isolate` sets `Options::isolate` and `--bcf-section-box` sets `Options::section_box`; neither is on by default.
- `src/ids.rs` is `check --ids` and `ids translate`: the document is translated by `axioval-ids` in memory, and a specification with any gap runs none of its rules (its folder is dropped) and is listed on stderr and in the result's additive `ids` field. A check with such a specification never exits 0 (4 without findings); `ids translate` exits 4 when it left one out. Both paths run the same packages, so `ids translate` then `check --ruleset` reports as `check --ids` does; `--ids-filter` hands a selector file to `Options::with_filter` and is recorded in the `ids` field; `tests/ids.rs` pins it, the storey prefilter included. `ids export` hands its packages to `axioval_ids::export`, lists every rule not exported on stderr, and exits 4 when it wrote the document without some, 1 (writing nothing) when none is exportable; `tests/ids.rs` pins a clash rule listed beside an exported property rule. The document is audited first (`TranslateError::Invalid`): any audit error exits 1, writes nothing and lists every finding (`digest::IdsAuditFinding`); warnings never refuse or change the status, and go to stderr and the `ids` field's additive `warnings`.
- `--bcf-snapshots` makes `geometry::attach` keep each meshed object's
  triangles (`GeometryReport::meshes`, `keep_meshes`), converted by
  `snapshot_meshes`, and exports through
  `bcf::export_with_snapshots` with `bcf_snapshot::Renderer`. Without
  `--geometry` it fails before writing anything; without the flag meshes
  are not kept and the archive is byte-identical.
- `tests/parity.rs` holds built-in capabilities to parity with their re-expressions on the pinned public models (`fixtures/parity`), through `check --geometry --rule-status`; it is ignored unless `AXIOVAL_PARITY_MODELS` names the fetched models, and then fails on any difference a case's `parity.json` does not record and on any recorded one no longer shown. Its non-ignored test compiles every case against the registry and checks every `recorded` rule has a recording for each model. A case's `recorded` rules (capabilities replaced by templates) are compared with the outcomes the replaced implementation reported, stored in `recorded/<model>.json`; `AXIOVAL_PARITY_RECORD=1` rewrites them, only before an implementation is replaced, never to make a difference disappear. Record a divergence with its reason and decision; never widen a comparison to make one disappear. A compared rule must judge something on some model: an outcome on either side, or objects its summary says it checked (a rule passing everywhere judges them, and the summaries are compared, #377); one judging nothing fails the case.
- `benches/templates.rs` is the template benchmark (#279, `docs/src/performance.md`): each `PAIRS` capability's rules run by the default registry and by one with its reference in its place (`CapabilityRegistry::replace`), on a generated fixture and the public models, meshed by the bridge the binary links (`axioval::ifc_geometry`; never fork it), held to `Parity::contract()` first. Its peak heap comes from the bench-only `peak_alloc` global allocator. `scripts/bench.py` runs and judges it; never fold it into `cargo test`. Its records list every run of both sides in order (`times_ns`, `peaks_bytes`, run `i` of each side paired) and `--only FILE` measures just the named `[capability, input]` pairs (a gate's further rounds, #293); keep both additive.
- `tests/check.rs` runs the real binary; keep a case for every exit status. Update `docs/src/cli.md` with any change to arguments, output or status.
- `check --locate` builds the IFC `LocationPolicy` (`IfcBuildingStorey`, `IfcSpace`, containment and aggregation, `Name`); `none` must leave the result byte-identical. `report --location` keeps outcomes whose location is unresolved.
- `check` identifies every finding with `Report::identify_findings` over
  `bcf::IFC_GLOBAL_ID_SCHEME`, the scheme the BCF sink keys topics by, so a
  decision and a topic GUID are one key. `--decisions` applies a decisions
  file (loaded before the run, so a bad file fails before any work);
  decisions never change the exit status. `decide` writes the decisions
  file with each finding's basis and writes nothing when any `--finding` is
  not in the result. `report` prints ids and decisions, counts them in the
  summary, lists stale ones as the `stale-decisions` section and filters by
  `--decision`.
- `check --decisions-from ARCHIVE` reads the BCF bytes before the run (a
  missing file fails before any work), imports them with `bcf::import`
  after identification and applies the decisions; it conflicts with
  `--decisions`. Unmatched topics go to the additive `unmatched_topics`
  field (`digest::UnmatchedTopicRecord`), the `unmatched-topics` section and
  one stderr warning; they never change the exit status. It imports with
  the run's `rule_labels`, so the export's own labels are not read back as
  a reviewer's.
- `decide --assign-to/--due/--priority/--label` keep their earlier values
  when a later decision does not restate them, and `--comment` appends to
  the finding's thread, never replaces it.
- `src/server.rs` is `axioval bcf push` and `pull`. It rebuilds the project
  from the result's `objects` (kind and GlobalId of every object the report
  names; resource objects stay with the report) and maps topics only
  through the sink and `axioval-bcf-api`. Credentials come from
  `AXIOVAL_BCF_TOKEN` or `AXIOVAL_BCF_CLIENT_SECRET`, never from arguments,
  and are never written. `tests/check.rs` includes the API crate's
  in-process server by path; never call a real server. HTTPS only with the
  `tls` feature.
- `--xlsx` (shared by `check` and `compare` through `OutputArgs`) writes
  `axioval-xlsx`'s workbook with the GlobalId scheme as the external
  identity column, created at `SOURCE_DATE_EPOCH` or now
  (`epoch_seconds`). It is built with every other output before anything
  is written; `tests/check.rs` reads two takeoff sheets back.
- `--html` (shared through `OutputArgs`) renders `axioval-html`'s report,
  dated as BCF topics are (`timestamp`). `--html-template` is read and
  parsed by `OutputArgs::prepare` at the start of `check` and `compare`,
  so a missing or refused template fails (status 1) before any work.
  Never run anything from a template; never print a PDF here.
- Packages are read only through `load_ruleset` and `load_definitions` in
  `main.rs`, which load the table files they name from the directory
  holding the package file; never parse a package with `load` alone, or a
  `tableFile` parameter reaches the binder unloaded.
- `check` reports every end of a declared relation's listed pair that names
  no single object (`unknown_relation_objects`, taken before the run) as an
  integrity record `relation-object-unknown` (warning, locator
  `axioval:derived.relation;id=<id>:pairs#row=<n>`); it never changes the
  exit status, the rules it leaves undecided do.
- `check --relations RELATION=FILE[#SHEET]` (`relation_arg`,
  `supply_relations`) reads each file with `SuppliedPairs::read` and
  supplies it to the plan right after compiling, before any model is read:
  an undeclared or non-supplied relation, a second file, an unreadable,
  oversized or refused file fails with status 1 and writes nothing. Each
  file is recorded in the additive `relation_files` field
  (`digest::RelationFileRecord`: relation, file name, SHA-256, pair count),
  and a supplied relation given no file is the integrity warning
  `relation-pairs-not-supplied`; its rules are not evaluated, never run
  over an empty relation.
- `check --catalogue CATALOGUE[@VERSION]=FILE[#SHEET]` (`catalogue_arg`,
  `supply_catalogues`) is the same for a ruleset's catalogues:
  `SuppliedCatalogue::read` with the argument's version, supplied right
  after compiling, the same refusals with status 1, each file recorded in
  the additive `catalogue_files` (`digest::CatalogueFileRecord`), and a
  catalogue given no file the integrity warning `catalogue-not-supplied`
  (locator `axioval:catalogue;id=<id>`); the objects reading it are not
  evaluated, never defaulted.
- `check --model-revision [DOCUMENT=]REVISION` (`src/results.rs`, #323):
  once the models are read, and only when a ruleset declares a results
  catalogue, each model's revision (`ModelRevision`: the SHA-256 of its
  file as given, plus the stated revision) is supplied for every source
  read from it (`ExecutionPlan::supply_model_revision`), and every
  `ExecutionPlan::stale_results` entry is the integrity warning
  `calculation-result-stale` or `calculation-result-untraceable` (locator
  `axioval:catalogue;id=<id>#entry=<key>`). A revision naming no model,
  stated twice, or bare beside several models fails with status 1.
- `check --zones FILE` (`src/zones.rs`, repeatable) reads each GeoJSON
  file with `ContextZones::read_geojson` and supplies it to the plan right
  after compiling (`ExecutionPlan::supply_context_zones`): an unreadable,
  oversized (64 MB) or refused file, or a zone id two files state, fails
  with status 1 and writes nothing. Each file is recorded in the additive
  `zone_files` (`digest::ZoneFileRecord`: file, SHA-256, reference system,
  zone ids). A plan reading zones (`ExecutionPlan::context_zone_reads`)
  with no file is the integrity warning `zones-not-supplied`, a zone it
  names that no file states `zone-not-supplied` (locator
  `axioval:zone;id=<id>`); the values reading them are not evaluated.
  `[ACCURACY=]FILE` (`zone_arg`): a file opening with `{` is GeoJSON and
  takes no accuracy, one opening with `<` is GML (`ContextZones::read_gml`:
  an `XPlanAuszug` is a zoning plan, #321, anything else a cadastre) and
  needs it; the record adds `format` and `accuracy_metres`, and a kind
  read by `stands_in` that no file states is `zone-kind-not-supplied`.
  After the models are read (`check_placements`, before the run) each
  file's `placements` record per source whether `ContextZones::placed_extent`
  places it (`model`/`matches`/`refused`, warning `zone-reference-mismatch`)
  and whether the placed extent meets the extent of the source's measured
  bodies (`overlap`/`apart` with `gap_metres`, warning
  `zones-apart-from-model`, or `unchecked` with the reason) (#319).
- `check --terrain KIND@ACCURACY=FILE[#SURFACE]` with `--terrain-in
  model|map` (`src/terrain.rs`: `terrain_arg`, `read_terrains`) reads each
  LandXML surface with `TerrainSurface::read_landxml` before any model is
  read and gives the runtime one `TinTerrain` (`Runtime::with_terrain`).
  The accuracy is required in the argument (LandXML states none) and the
  frame by `--terrain-in`: never default either. A second file for one
  ground, an unreadable, oversized or refused file fails with status 1 and
  writes nothing; each file is recorded in the additive `terrain_files`
  (`digest::TerrainFileRecord`). A ground the plan reads
  (`terrain_kinds_read`) that no file supplies is the integrity warning
  `terrain-not-supplied`; its values are not evaluated, never measured
  against the other ground. `tests/terrain.rs` runs it end to end.
- `check --timings` prints each phase's time on stderr as it ends (`src/timings.rs`, `Stopwatch`; the bridge's phases through `Options::with_timings`; rules through `Runtime::with_rule_observer`) and never changes the result or status; `tests/check.rs` pins it. Envelope declarations (`IsExternal` of every meshed object) are read on the first envelope membership request (`LazyEnvelope`), never eagerly: a ruleset without an envelope rule resolves none (the same test).
- `check --rule-status` records `Report::rules`; the summary lists them bounded by `--top` (rules that did not pass first) and never changes the exit status.
- `check --rule-timings` (`Runtime::with_rule_timings`, implying `--rule-status`) adds each rule's `timing` to its summary; the summary's `slowest` lists the slowest rules bounded by `--top`. Without the `timing` fields the result equals the `--rule-status` one (`tests/check.rs` pins it); never let a timing change a finding, an identity or the exit status. `--timings` stays stderr only.
