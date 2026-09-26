# Report sinks

A sink turns a finished `Report` into a format other tools read. Sinks depend
on `axioval-ir` alone: they see findings, not-evaluated outcomes and the
project the report was computed over, never the engine or a source adapter.

## BCF

`axioval-bcf` writes BCF 2.1 archives through `openbim-bcf`.

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
| Finding | `TopicType` is the severity (`Error`, `Warning`, `Info`); the label is the rule id |
| Not-evaluated outcome | `TopicType` is `Not evaluated`; the description names the reason |

Every topic's description repeats the message, the rule, the source-qualified
object ids and each evidence locator with its exactness.

**Not-evaluated outcomes are exported by default.** An archive that lists only
findings reads as "everything else passed". `Options::include_not_evaluated`
turns them off explicitly.

**Components are selected by GlobalId**, read from the `ifc-globalid` alias the
IFC adapter attaches. The subject comes first, then related objects. An
object without the alias cannot be selected: its topic is still written, and
it is listed in `Export::unanchored`. A viewpoint never selects related
objects alone, because it would point the reviewer at the wrong element.

**GUIDs are stable across re-exports.** Topic and viewpoint GUIDs are UUIDv5
over the rule, the objects' GlobalIds and the message, so rechecking a revised
model reproduces the GUID of every issue still present, and BCF tools can
track it. Objects without a GlobalId fall back to their STEP-numbered identity
and cannot be tracked this way. When a federation holds two revisions that
produce the same issue, those topics are qualified by source to stay unique.

**Output is deterministic.** The caller supplies author and date, nothing reads
the clock, and identical input writes identical bytes. Written archives are
checked against the buildingSMART 2.1 schemas.

BCF 3.0 is not written: it requires a camera on every viewpoint, and a report
carries no geometry to place one.
