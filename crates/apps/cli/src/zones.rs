//! `check --zones`: context zones supplied beside the model (#327), read
//! from `GeoJSON` files and supplied to the plan before anything runs.

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use axioval::engine::{
    ContextZones, ExecutionPlan, MODEL_REFERENCE, ZONE_LOCATOR_PREFIX, ZoneReference,
};

use crate::digest::{IntegrityRecord, ZoneFileRecord};

/// The largest zone file read, in bytes.
const ZONE_FILE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// Reads each `--zones` file and supplies its zones to `plan`, before
/// anything runs: the record of each file, and an integrity warning for
/// every zone the plan reads that none supplied ([`unsupplied`]).
///
/// # Errors
///
/// A file that cannot be read, is too large or is refused (no reference
/// system, a zone without an id, kind, accuracy or readable geometry), and
/// a zone id two files state.
pub(crate) fn supply(
    plan: &mut ExecutionPlan,
    paths: &[PathBuf],
) -> Result<(Vec<ZoneFileRecord>, Vec<IntegrityRecord>), Box<dyn Error>> {
    let mut records = Vec::new();
    for path in paths {
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
        let zones = ContextZones::read_geojson(&bytes, name).map_err(refuse)?;
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
    reads
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
        })
        .collect()
}
