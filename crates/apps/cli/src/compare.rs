//! `axioval compare`: two revisions of a model, object by object.
//!
//! Each revision is imported as its own session and matched by IFC
//! `GlobalId` through `axioval_rules::compare_sessions`, its related objects
//! compared per relationship kind (containment, fills, ...). The comparison is
//! projected into an ordinary report, one rule id per facet, so the saved
//! result reads back with `axioval report` and travels through the BCF sink
//! like a check's; the structured comparison rides beside it in the
//! result's `comparison` field.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use axioval::bcf_snapshot;
use axioval::engine::EvidenceSession;
use axioval::ifc::IFC_GLOBAL_ID;
use axioval::ir::{Object, ObjectId, Project, RuleId, Severity};
use axioval::rules::{
    ComparisonRequest, ComparisonTolerance, Difference, Facet, GeometryMode, Measurement,
    ModelComparison, ObjectChange, Side, Unresolved, compare_sessions,
};
use clap::Args;

use crate::digest::{
    AmbiguousRecord, ChangeRecord, CheckOutput, ComparedRecord, ComparisonCounts, ComparisonRecord,
    GapRecord, GeometryRecord, SideObject, SourceRecord, ToleranceRecord, Unmeasured,
    WitnessRecord,
};
use crate::{Outcome, OutputArgs, emit, geometry, integrity};

/// The rule id every comparison entry's rule id starts with.
const RULE: &str = "compare";

#[derive(Args)]
#[allow(clippy::struct_excessive_bools)] // Each is one independent flag.
pub struct CompareArgs {
    /// The earlier revision: an IFC2X3, IFC4 or IFC4X3 STEP file, or an
    /// ifcZIP archive holding one.
    #[arg(long, value_name = "PATH")]
    base: PathBuf,
    /// The later revision of the same model.
    #[arg(long, value_name = "PATH")]
    revised: PathBuf,
    /// Also compare a property, as `SET.NAME` (split at the first `.`) or a
    /// bare `NAME`, resolved exactly on both sides. Repeat for several.
    #[arg(long = "property", value_name = "SET.NAME")]
    properties: Vec<String>,
    /// Also compare every property of a set, listed on both sides. Repeat
    /// for several.
    #[arg(long = "property-set", value_name = "SET")]
    property_sets: Vec<String>,
    /// Also compare every property of every set, listed on both sides.
    #[arg(long)]
    all_property_sets: bool,
    /// Also compare the header timestamps: a revised file written before
    /// the base is an error finding.
    #[arg(long)]
    timestamps: bool,
    /// Also mesh both revisions and compare each object's body: its
    /// measured bounds, or as `--geometry-mode` says.
    #[arg(long)]
    geometry: bool,
    /// How `--geometry` compares bodies: `bounds`, the largest shift of any
    /// face of the bounds, or `mesh`, the certified distance between the
    /// two surfaces, which also sees a reshaping inside unchanged bounds.
    /// Where both revisions of a body have an exact boundary, `mesh`
    /// measures between the boundaries, curved bodies included.
    #[arg(
        long,
        value_name = "MODE",
        default_value = "bounds",
        value_parser = ["bounds", "mesh"],
        requires = "geometry"
    )]
    geometry_mode: String,
    /// With `--geometry-mode mesh`, mesh only: build no exact boundaries.
    /// By default a body whose construction is exact (a vertically placed
    /// extrusion of a rectangle, circle, section or line-and-arc profile)
    /// also gets its exact boundary, so a curved body's surface distance is
    /// certified between the boundaries instead of left open by its
    /// tessellation.
    #[arg(long, requires = "geometry")]
    no_exact_boundaries: bool,
    /// Largest length difference that counts as unchanged, in metres.
    #[arg(long, default_value_t = 0.005, value_name = "METRES")]
    length_tolerance: f64,
    /// Largest angle difference that counts as unchanged, in degrees.
    #[arg(long, default_value_t = 0.01, value_name = "DEGREES")]
    angle_tolerance: f64,
    #[command(flatten)]
    output: OutputArgs,
}

/// Both revisions' geometry as one record, and their kept meshes.
fn combined(
    before: geometry::GeometryReport,
    after: geometry::GeometryReport,
) -> (GeometryRecord, BTreeMap<ObjectId, bcf_snapshot::Mesh>) {
    let mut unmeasured: Vec<Unmeasured> = before
        .unmeasured
        .into_iter()
        .chain(after.unmeasured)
        .map(|(object, reason)| Unmeasured { object, reason })
        .collect();
    unmeasured.sort_by(|a, b| a.object.cmp(&b.object));
    let mut bodies = before.meshes;
    bodies.extend(after.meshes);
    let record = GeometryRecord {
        exact: before.exact + after.exact,
        tessellated: before.tessellated + after.tessellated,
        no_body: before.no_body + after.no_body,
        exact_boundaries: before.exact_boundaries + after.exact_boundaries,
        composed: before.composed + after.composed,
        unmeasured,
    };
    (record, bodies)
}

pub fn compare(mut args: CompareArgs) -> Result<Outcome, Box<dyn Error>> {
    args.output.prepare()?;
    let tolerance =
        ComparisonTolerance::try_new(args.length_tolerance, args.angle_tolerance.to_radians())
            .map_err(|error| format!("--length-tolerance/--angle-tolerance: {error}"))?;
    let mut request = ComparisonRequest::new(IFC_GLOBAL_ID)?
        .with_relationships()
        .with_placement(tolerance)
        .with_coordinate_systems(tolerance);
    if args.geometry {
        request = if args.geometry_mode == "mesh" {
            request.with_mesh_geometry(tolerance)
        } else {
            request.with_geometry(tolerance)
        };
    }
    for set in &args.property_sets {
        request = request
            .with_property_set(set)
            .map_err(|error| format!("--property-set `{set}`: {error}"))?;
    }
    if args.all_property_sets {
        request = request.with_all_property_sets();
    }
    if args.timestamps {
        request = request.with_timestamps();
    }
    for property in &args.properties {
        let (set, name) = match property.split_once('.') {
            Some((set, name)) => (Some(set), name),
            None => (None, property.as_str()),
        };
        request = request
            .with_property(set, name)
            .map_err(|error| format!("--property `{property}`: {error}"))?;
    }

    let (base_name, revised_name) = documents(&args.base, &args.revised)?;
    let (base, base_bytes) = import(&args.base, &base_name)?;
    let (revised, revised_bytes) = import(&args.revised, &revised_name)?;
    let project = Project::new(
        base.project()
            .objects()
            .chain(revised.project().objects())
            .cloned()
            .collect(),
    )
    .map_err(|error| format!("the revisions cannot be listed together: {error}"))?;
    let mut records = integrity(&base)?;
    records.extend(integrity(&revised)?);

    let (base, revised, meshed) = if args.geometry {
        // Only the surface distance reads exact boundaries: bounds never do.
        let keep = geometry::Options::meshes(args.output.bcf_view.snapshots)
            .with_exact_boundaries(args.geometry_mode == "mesh" && !args.no_exact_boundaries);
        let (base, before) = geometry::attach(base, &base_bytes, keep)
            .map_err(|error| format!("geometry of {}: {error}", args.base.display()))?;
        let (revised, after) = geometry::attach(revised, &revised_bytes, keep)
            .map_err(|error| format!("geometry of {}: {error}", args.revised.display()))?;
        (base, revised, Some(combined(before, after)))
    } else {
        (base, revised, None)
    };
    let (meshed, bodies) = match meshed {
        Some((record, kept)) => (Some(record), Some(kept)),
        None => (None, None),
    };

    let comparison = compare_sessions(&base, &revised, &request);
    let report = comparison.report(&RuleId::new(RULE)?, &Severity::Warning);
    let record = record(
        &comparison,
        &project,
        (&base, &revised),
        &request,
        tolerance,
    );
    let output = CheckOutput::new(report, records, meshed, &project).with_comparison(record);
    let bounds = args
        .geometry
        .then(|| geometry::bounds(&[&base, &revised], &output.report));
    // A comparison runs no ruleset, so its topics have no rule labels.
    emit(
        &output,
        &project,
        (bounds, bodies),
        BTreeMap::new(),
        args.output,
    )?;
    Ok(Outcome::of(&output.report))
}

/// The document names of the two sources: their file names, or, when both
/// files share one, that name marked `@base` and `@revised`, so the two
/// revisions stay two sources and every object id stays unique.
fn documents(base: &Path, revised: &Path) -> Result<(String, String), Box<dyn Error>> {
    let name = |path: &Path| -> Result<String, Box<dyn Error>> {
        Ok(path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{}: model path has no UTF-8 file name", path.display()))?
            .to_owned())
    };
    let (before, after) = (name(base)?, name(revised)?);
    Ok(if before == after {
        (format!("{before}@base"), format!("{after}@revised"))
    } else {
        (before, after)
    })
}

fn import(
    path: &Path,
    document: &str,
) -> Result<(EvidenceSession, geometry::ModelBytes), Box<dyn Error>> {
    let (session, content) = crate::import(path, Some(document))?;
    let mut bytes = geometry::ModelBytes::new();
    for snapshot in session.snapshots() {
        bytes.insert(snapshot.source().clone(), content.clone());
    }
    Ok((session, bytes))
}

fn side(side: Side) -> String {
    match side {
        Side::Base => "base",
        Side::Revised => "revised",
    }
    .to_owned()
}

fn change(difference: &Difference) -> ChangeRecord {
    match difference {
        Difference::Measured(measurement) => measured(measurement),
        other => ChangeRecord {
            facet: other.facet().name().to_owned(),
            detail: other.to_string(),
            measure: None,
            lower: None,
            upper: None,
            tolerance: None,
            unit: None,
            witness: None,
        },
    }
}

fn measured(measurement: &Measurement) -> ChangeRecord {
    ChangeRecord {
        facet: measurement.measure.facet().name().to_owned(),
        detail: measurement.to_string(),
        measure: Some(measurement.measure.name().to_owned()),
        lower: Some(measurement.lower),
        upper: Some(measurement.upper),
        tolerance: Some(measurement.tolerance),
        unit: Some(measurement.measure.unit().to_owned()),
        witness: measurement.witness.map(|witness| WitnessRecord {
            side: side(witness.side),
            from: witness.from,
            to: witness.to,
        }),
    }
}

fn gap(entry: &Unresolved) -> GapRecord {
    GapRecord {
        facet: entry.facet.name().to_owned(),
        detail: entry.to_string(),
    }
}

fn kind(project: &Project, id: &ObjectId) -> String {
    project
        .object(id)
        .map(Object::kind)
        .unwrap_or_default()
        .to_owned()
}

/// Every identity that is not unchanged, and the counts of all of them.
fn objects(
    comparison: &ModelComparison,
    project: &Project,
) -> (Vec<ComparedRecord>, ComparisonCounts) {
    let mut counts = ComparisonCounts {
        unidentified: comparison.unidentified().len(),
        ambiguous: comparison.ambiguous().len(),
        ..ComparisonCounts::default()
    };
    let mut objects = Vec::new();
    for compared in comparison.objects() {
        let entry =
            |state: &str, kind: String, base: Option<&ObjectId>, revised: Option<&ObjectId>| {
                ComparedRecord {
                    identity: compared.identity.clone(),
                    state: state.into(),
                    kind,
                    base: base.cloned(),
                    revised: revised.cloned(),
                    changes: Vec::new(),
                    unresolved: Vec::new(),
                    undetermined: Vec::new(),
                }
            };
        objects.push(match &compared.change {
            ObjectChange::Added { revised } => {
                counts.added += 1;
                entry("added", kind(project, revised), None, Some(revised))
            }
            ObjectChange::Removed { base } => {
                counts.removed += 1;
                entry("removed", kind(project, base), Some(base), None)
            }
            ObjectChange::Matched {
                base,
                revised,
                differences,
                unresolved,
                undetermined,
            } => {
                let state = if !differences.is_empty() {
                    counts.changed += 1;
                    "changed"
                } else if !unresolved.is_empty() || !undetermined.is_empty() {
                    counts.incomplete += 1;
                    "incomplete"
                } else {
                    counts.unchanged += 1;
                    continue;
                };
                ComparedRecord {
                    changes: differences.iter().map(change).collect(),
                    unresolved: unresolved.iter().map(gap).collect(),
                    undetermined: undetermined.iter().map(measured).collect(),
                    ..entry(state, kind(project, revised), Some(base), Some(revised))
                }
            }
        });
    }
    (objects, counts)
}

/// Coordinate systems per pair of sources, then every unpaired source.
fn coordinate_systems(comparison: &ModelComparison) -> Vec<SourceRecord> {
    let paired = comparison.sources().iter().map(|pair| SourceRecord {
        base: Some(pair.base.clone()),
        revised: Some(pair.revised.clone()),
        changes: pair.differences.iter().map(change).collect(),
        unresolved: pair.unresolved.iter().map(gap).collect(),
        undetermined: pair.undetermined.iter().map(measured).collect(),
    });
    let unpaired = comparison
        .unpaired_sources()
        .iter()
        .map(|(which, id)| SourceRecord {
            base: (*which == Side::Base).then(|| id.clone()),
            revised: (*which == Side::Revised).then(|| id.clone()),
            changes: Vec::new(),
            unresolved: vec![GapRecord {
                facet: Facet::CoordinateSystem.name().to_owned(),
                detail: "no counterpart source".into(),
            }],
            undetermined: Vec::new(),
        });
    paired.chain(unpaired).collect()
}

/// The facets `request` compares, in facet order.
fn facets(request: &ComparisonRequest) -> Vec<String> {
    let mut facets = vec![
        Facet::Kind,
        Facet::Classifications,
        Facet::Property,
        Facet::Relationship,
    ];
    for (facet, requested) in [
        (Facet::Placement, request.placement()),
        (Facet::Geometry, request.geometry()),
        (Facet::CoordinateSystem, request.coordinate_systems()),
    ] {
        if requested.is_some() {
            facets.push(facet);
        }
    }
    if request.compares_timestamps() {
        facets.push(Facet::Timestamp);
    }
    facets
        .into_iter()
        .map(|facet| facet.name().to_owned())
        .collect()
}

fn record(
    comparison: &ModelComparison,
    project: &Project,
    (base, revised): (&EvidenceSession, &EvidenceSession),
    request: &ComparisonRequest,
    tolerance: ComparisonTolerance,
) -> ComparisonRecord {
    let source = |session: &EvidenceSession| {
        session
            .snapshots()
            .map(|snapshot| snapshot.source().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (objects, counts) = objects(comparison, project);
    ComparisonRecord {
        base: source(base),
        revised: source(revised),
        scheme: comparison.scheme().to_owned(),
        facets: facets(request),
        geometry_mode: (request.geometry().is_some()
            && request.geometry_mode() == GeometryMode::Mesh)
            .then(|| GeometryMode::Mesh.name().to_owned()),
        tolerance: ToleranceRecord {
            length_metres: tolerance.length_metres(),
            angle_degrees: tolerance.angle_radians().to_degrees(),
        },
        counts,
        objects,
        unidentified: comparison
            .unidentified()
            .iter()
            .map(|(which, object)| SideObject {
                side: side(*which),
                object: object.clone(),
            })
            .collect(),
        ambiguous: comparison
            .ambiguous()
            .iter()
            .map(|entry| AmbiguousRecord {
                side: side(entry.side),
                identity: entry.identity.clone(),
                objects: entry.objects.clone(),
            })
            .collect(),
        coordinate_systems: coordinate_systems(comparison),
    }
}
