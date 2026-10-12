//! `check --zones`: context zones supplied beside the model (#327), read
//! from `GeoJSON` files or GML documents (cadastral parcels, #319) and
//! supplied to the plan before anything runs; once the models are read,
//! each file's reference system and coordinates are compared with each
//! model's (#319).

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use axioval::engine::{
    ContextZones, CoordinateSystemServiceHandle, EvidenceSession, ExecutionPlan, MODEL_REFERENCE,
    ProximityServiceHandle, ZONE_LOCATOR_PREFIX, ZoneReference,
};

use crate::digest::{IntegrityRecord, ZoneFileRecord, ZonePlacementRecord};

/// The largest zone file read, in bytes.
const ZONE_FILE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// One `--zones` argument: the file, and the accuracy of its boundaries
/// where the file states none.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ZoneArg {
    accuracy_metres: Option<f64>,
    path: PathBuf,
}

/// Parses `[ACCURACY=]FILE`: the accuracy in metres (a trailing `m`
/// allowed) where the text before the first `=` is one, else the whole
/// argument is the file.
pub(crate) fn zone_arg(value: &str) -> Result<ZoneArg, String> {
    if let Some((head, file)) = value.split_once('=') {
        let number = head.trim().strip_suffix('m').unwrap_or(head.trim());
        if let Ok(accuracy) = number.parse::<f64>() {
            if !accuracy.is_finite() || accuracy < 0.0 || file.is_empty() {
                return Err(
                    "expected [ACCURACY=]FILE: ACCURACY the boundaries' accuracy in metres, at \
                     least zero"
                        .to_owned(),
                );
            }
            return Ok(ZoneArg {
                accuracy_metres: Some(accuracy),
                path: PathBuf::from(file),
            });
        }
    }
    if value.is_empty() {
        return Err("expected [ACCURACY=]FILE".to_owned());
    }
    Ok(ZoneArg {
        accuracy_metres: None,
        path: PathBuf::from(value),
    })
}

/// The zones of `bytes`, by what the file is: a `GeoJSON` collection
/// stating its own accuracy, or a GML document whose accuracy the
/// argument states.
fn read(bytes: &[u8], name: &str, accuracy: Option<f64>) -> Result<ContextZones, String> {
    let text = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match (
        text.iter().find(|byte| !byte.is_ascii_whitespace()),
        accuracy,
    ) {
        (Some(b'{'), None) => ContextZones::read_geojson(bytes, name),
        (Some(b'{'), Some(_)) => Err(
            "a GeoJSON file states the accuracy of its zones itself; give it as FILE alone"
                .to_owned(),
        ),
        (Some(b'<'), Some(accuracy)) => ContextZones::read_gml(bytes, name, accuracy),
        (Some(b'<'), None) => Err(
            "a GML document states no accuracy of its boundaries; give it as \
             ACCURACY=FILE, in metres"
                .to_owned(),
        ),
        _ => Err("it is neither a GeoJSON feature collection nor a GML document".to_owned()),
    }
}

/// Reads each `--zones` file and supplies its zones to `plan`, before
/// anything runs: the record of each file, and an integrity warning for
/// every zone the plan reads that none supplied ([`unsupplied`]).
///
/// # Errors
///
/// A file that cannot be read, is too large or is refused (no reference
/// system, a zone without an id, kind, accuracy or readable geometry, a
/// feature outside a reader's subset), and a zone id two files state.
pub(crate) fn supply(
    plan: &mut ExecutionPlan,
    args: &[ZoneArg],
) -> Result<(Vec<ZoneFileRecord>, Vec<IntegrityRecord>), Box<dyn Error>> {
    let mut records = Vec::new();
    for arg in args {
        let path = &arg.path;
        let refuse = |detail: String| format!("--zones {}: {detail}", path.display());
        let size = fs::metadata(path)
            .map_err(|error| refuse(error.to_string()))?
            .len();
        if size > ZONE_FILE_LIMIT_BYTES {
            return Err(refuse(format!(
                "the file is larger than {ZONE_FILE_LIMIT_BYTES} bytes"
            ))
            .into());
        }
        let bytes = fs::read(path).map_err(|error| refuse(error.to_string()))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| refuse("the file name is not UTF-8".into()))?;
        let zones = read(&bytes, name, arg.accuracy_metres).map_err(refuse)?;
        records.push(ZoneFileRecord {
            file: zones.origin().to_owned(),
            sha256: zones.sha256().to_owned(),
            reference: match zones.reference() {
                ZoneReference::Model => MODEL_REFERENCE.to_owned(),
                ZoneReference::Map(name) => name.clone(),
            },
            zones: zones
                .zones()
                .iter()
                .map(|zone| zone.id().to_owned())
                .collect(),
            format: zones.format().map(str::to_owned),
            accuracy_metres: arg.accuracy_metres,
            placements: Vec::new(),
        });
        plan.supply_context_zones(zones)
            .map_err(|error| refuse(error.to_string()))?;
    }
    Ok((records, unsupplied(plan)))
}

/// Every context zone the plan reads that no file supplied, as an
/// integrity warning: the values reading it are not evaluated.
fn unsupplied(plan: &ExecutionPlan) -> Vec<IntegrityRecord> {
    let reads = plan.context_zone_reads();
    let supplied = plan.context_zones();
    if !supplied.is_supplied() && reads.any {
        return vec![IntegrityRecord {
            code: "zones-not-supplied".into(),
            severity: "warning".into(),
            message: "the rules read context zones (parcels, zoning areas, building lines) \
                      beside the model, and none were supplied (`--zones FILE`); every value \
                      reading one is not evaluated"
                .into(),
            locator: ZONE_LOCATOR_PREFIX.into(),
        }];
    }
    let ids = reads
        .zones
        .iter()
        .filter(|id| supplied.is_supplied() && supplied.zone(id).is_none())
        .map(|id| IntegrityRecord {
            code: "zone-not-supplied".into(),
            severity: "warning".into(),
            message: format!(
                "the rules read context zone `{id}`, which no `--zones` file states; every \
                 value reading it is not evaluated"
            ),
            locator: format!("{ZONE_LOCATOR_PREFIX};id={id}"),
        });
    let kinds = reads
        .kinds
        .iter()
        .filter(|kind| supplied.is_supplied() && supplied.of_kind(kind).next().is_none())
        .map(|kind| IntegrityRecord {
            code: "zone-kind-not-supplied".into(),
            severity: "warning".into(),
            message: format!(
                "the rules read the zone of kind `{kind}` an object stands in, and no \
                 `--zones` file states a zone of that kind; every value reading one is not \
                 evaluated"
            ),
            locator: format!("{ZONE_LOCATOR_PREFIX};stands_in={kind}"),
        });
    ids.chain(kinds).collect()
}

/// The plan extent `[west, south, east, north]` of every measured body
/// of `source`, grown by each one's chord deviation, or why there is none.
fn model_extent(
    session: &EvidenceSession,
    source: &axioval::ir::SourceId,
) -> Result<[f64; 4], String> {
    let service = session
        .service::<ProximityServiceHandle>()
        .ok_or("no geometry is measured (`--geometry`), so the model's extent is unknown")?;
    let mut extent: Option<[f64; 4]> = None;
    for object in session.project().objects() {
        if object.id.source != *source {
            continue;
        }
        let Ok(bounds) = service.bounds(&object.id) else {
            continue;
        };
        let (min, max) = (bounds.enclosing().min(), bounds.enclosing().max());
        let [west, south, east, north] = extent.get_or_insert([
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ]);
        *west = west.min(min[0]);
        *south = south.min(min[1]);
        *east = east.max(max[0]);
        *north = north.max(max[1]);
    }
    extent.ok_or_else(|| "no body of the model is measured, so its extent is unknown".to_owned())
}

/// Records in `records` (one per file of `plan`'s zones, in order)
/// whether each file's reference system and coordinates match each
/// model's of `session`, and warns of each pair that does not
/// (`zone-reference-mismatch`, `zones-apart-from-model`): a value reading
/// such a zone is not evaluated, or measures zones that lie elsewhere.
pub(crate) fn check_placements(
    session: &EvidenceSession,
    plan: &ExecutionPlan,
    records: &mut [ZoneFileRecord],
) -> Vec<IntegrityRecord> {
    let systems = session.service::<CoordinateSystemServiceHandle>();
    let mut warnings = Vec::new();
    for (file, record) in plan.context_zones().files().iter().zip(records.iter_mut()) {
        for snapshot in session.snapshots() {
            let source = snapshot.source();
            let system = match (file.reference(), &systems) {
                (ZoneReference::Model, _) => Ok(None),
                (ZoneReference::Map(_), Some(systems)) => systems
                    .coordinate_system(source)
                    .map(Some)
                    .map_err(|error| {
                        format!("the model's coordinate system cannot be read: {error}")
                    }),
                (ZoneReference::Map(_), None) => {
                    Err("no coordinate-system service states the model's".to_owned())
                }
            };
            let placed = system.and_then(|system| file.placed_extent(system.as_ref()));
            let reference = match (file.reference(), &placed) {
                (_, Err(_)) => "refused",
                (ZoneReference::Model, Ok(_)) => "model",
                (ZoneReference::Map(_), Ok(_)) => "matches",
            };
            let mut entry = ZonePlacementRecord {
                source: source.to_string(),
                reference: reference.to_owned(),
                coordinates: "unchecked".to_owned(),
                gap_metres: None,
                reason: None,
            };
            match placed {
                Err(why) => {
                    warnings.push(IntegrityRecord {
                        code: "zone-reference-mismatch".into(),
                        severity: "warning".into(),
                        message: format!(
                            "the zones of {} cannot be placed in the coordinates of {source}: \
                             {why}; every value reading one of them there is not evaluated",
                            record.file
                        ),
                        locator: format!("{ZONE_LOCATOR_PREFIX};sha256={}", record.sha256),
                    });
                    entry.reason = Some(why);
                }
                Ok(None) => entry.reason = Some("the file states no zone".to_owned()),
                Ok(Some(zones)) => match model_extent(session, source) {
                    Err(why) => entry.reason = Some(why),
                    Ok(model) => {
                        let gap_x = (zones[0] - model[2]).max(model[0] - zones[2]).max(0.0);
                        let gap_y = (zones[1] - model[3]).max(model[1] - zones[3]).max(0.0);
                        let gap = gap_x.hypot(gap_y);
                        if gap > 0.0 {
                            "apart".clone_into(&mut entry.coordinates);
                            entry.gap_metres = Some(gap);
                            warnings.push(IntegrityRecord {
                                code: "zones-apart-from-model".into(),
                                severity: "warning".into(),
                                message: format!(
                                    "the zones of {} lie {gap:.1} m apart from every body of \
                                     {source} once placed in its coordinates: their \
                                     coordinates or the model's georeferencing do not match",
                                    record.file
                                ),
                                locator: format!("{ZONE_LOCATOR_PREFIX};sha256={}", record.sha256),
                            });
                        } else {
                            "overlap".clone_into(&mut entry.coordinates);
                        }
                    }
                },
            }
            record.placements.push(entry);
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zone_argument_names_its_file_and_perhaps_its_accuracy() {
        assert_eq!(
            zone_arg("zones.geojson"),
            Ok(ZoneArg {
                accuracy_metres: None,
                path: PathBuf::from("zones.geojson"),
            })
        );
        assert_eq!(
            zone_arg("0.05m=dir/alkis.xml"),
            Ok(ZoneArg {
                accuracy_metres: Some(0.05),
                path: PathBuf::from("dir/alkis.xml"),
            })
        );
        assert_eq!(
            zone_arg("a=b.xml"),
            Ok(ZoneArg {
                accuracy_metres: None,
                path: PathBuf::from("a=b.xml"),
            })
        );
        for refused in ["", "-1=alkis.xml", "0.1=", "NaN=a.xml"] {
            assert!(zone_arg(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn a_file_is_read_by_what_it_is() {
        assert!(
            read(b"  {}", "a", Some(0.1))
                .unwrap_err()
                .contains("itself")
        );
        assert!(
            read(b"<a/>", "a", None)
                .unwrap_err()
                .contains("ACCURACY=FILE")
        );
        assert!(read(b"x", "a", None).unwrap_err().contains("neither"));
    }
}
