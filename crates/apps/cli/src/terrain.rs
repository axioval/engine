//! `check --terrain`: terrain surfaces read as context beside the models.
//!
//! Each argument names which ground a `LandXML` surface is (`original` or
//! `finished`), its vertical accuracy (`LandXML` states none, so the argument
//! must), and the file with the surface to read; `--terrain-in` says
//! whether the files are in the models' own coordinates or in the map
//! their stated map conversions map onto. Every file is read before any
//! model, and one that cannot be read stops the check.

use std::{error::Error, fs, path::PathBuf, sync::Arc};

use axioval::engine::{
    ExecutionPlan, TERRAIN_FILE_LIMIT_BYTES, TERRAIN_LOCATOR_PREFIX, TerrainFrame, TerrainKind,
    TerrainServiceHandle, TerrainSurface, TinTerrain, terrain_kinds_read,
};

use crate::digest::{IntegrityRecord, TerrainFileRecord};

/// What `--terrain` gave a check: the service the run measures against
/// (`None` without any), what each file was, and a warning for each ground
/// the plan reads but no file supplied.
pub(crate) struct Terrains {
    pub(crate) service: Option<TerrainServiceHandle>,
    pub(crate) files: Vec<TerrainFileRecord>,
    pub(crate) warnings: Vec<IntegrityRecord>,
}

/// One `--terrain` argument.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainArg {
    kind: TerrainKind,
    accuracy_metres: f64,
    path: PathBuf,
    surface: Option<String>,
}

/// The coordinates `--terrain` files are given in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum TerrainIn {
    /// The models' own coordinates.
    Model,
    /// The map each model's stated map conversion maps onto.
    Map,
}

impl TerrainIn {
    fn frame(self) -> TerrainFrame {
        match self {
            Self::Model => TerrainFrame::Model,
            Self::Map => TerrainFrame::Map,
        }
    }
}

/// Parses `KIND@ACCURACY=FILE[#SURFACE]`: `original` or `finished`, the
/// vertical accuracy in metres (a trailing `m` allowed), the `LandXML` file
/// and, after its last `#`, the surface's name.
pub(crate) fn terrain_arg(value: &str) -> Result<TerrainArg, String> {
    let refused = || {
        "expected KIND@ACCURACY=FILE[#SURFACE]: KIND `original` or `finished`, ACCURACY the \
         surface's vertical accuracy in metres (LandXML states none)"
            .to_owned()
    };
    let (head, file) = value.split_once('=').ok_or_else(refused)?;
    let (kind, accuracy) = head.split_once('@').ok_or_else(refused)?;
    let kind = TerrainKind::parse(kind).ok_or_else(refused)?;
    let accuracy = accuracy.strip_suffix('m').unwrap_or(accuracy);
    let accuracy_metres: f64 = accuracy.trim().parse().map_err(|_| refused())?;
    if !accuracy_metres.is_finite() || accuracy_metres < 0.0 || file.is_empty() {
        return Err(refused());
    }
    let (path, surface) = match file.rsplit_once('#') {
        Some((path, surface)) if !path.is_empty() && !surface.is_empty() => {
            (path, Some(surface.to_owned()))
        }
        _ => (file, None),
    };
    Ok(TerrainArg {
        kind,
        accuracy_metres,
        path: PathBuf::from(path),
        surface,
    })
}

/// Reads every `--terrain` file as a surface in the frame `within`
/// states, and warns of each ground `plan` reads that none supplies
/// (`terrain-not-supplied`).
///
/// # Errors
///
/// A kind given twice, and a file that cannot be read, is too large, or
/// whose surface is refused.
pub(crate) fn read_terrains(
    args: &[TerrainArg],
    within: Option<TerrainIn>,
    plan: &ExecutionPlan,
) -> Result<Terrains, Box<dyn Error>> {
    let supplied: Vec<TerrainKind> = args.iter().map(|arg| arg.kind).collect();
    let warnings = terrain_kinds_read(plan)
        .into_iter()
        .filter(|kind| !supplied.contains(kind))
        .map(|kind| IntegrityRecord {
            code: "terrain-not-supplied".into(),
            severity: "warning".into(),
            message: format!(
                "the rules measure heights above the {kind} terrain, and none was supplied \
                 (`--terrain {kind}@ACCURACY=FILE`); every such value is not evaluated"
            ),
            locator: format!("{TERRAIN_LOCATOR_PREFIX};kind={kind}"),
        })
        .collect();
    if args.is_empty() {
        return Ok(Terrains {
            service: None,
            files: Vec::new(),
            warnings,
        });
    }
    let frame = within
        .ok_or("`--terrain` needs `--terrain-in model|map`: the coordinates its files are in")?
        .frame();
    let mut surfaces = Vec::new();
    let mut records = Vec::new();
    for arg in args {
        let shown = arg.path.display();
        let refuse = |detail: String| format!("--terrain {}={shown}: {detail}", arg.kind);
        if surfaces
            .iter()
            .any(|surface: &TerrainSurface| surface.kind() == arg.kind)
        {
            return Err(refuse(format!("a second {} terrain", arg.kind)).into());
        }
        let size = fs::metadata(&arg.path)
            .map_err(|error| refuse(error.to_string()))?
            .len();
        if size > TERRAIN_FILE_LIMIT_BYTES {
            return Err(refuse(format!(
                "the file is larger than {TERRAIN_FILE_LIMIT_BYTES} bytes"
            ))
            .into());
        }
        let bytes = fs::read(&arg.path).map_err(|error| refuse(error.to_string()))?;
        let name = arg
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| refuse("the file name is not UTF-8".into()))?;
        let surface = TerrainSurface::read_landxml(
            &bytes,
            name,
            arg.surface.as_deref(),
            arg.kind,
            arg.accuracy_metres,
            frame,
        )
        .map_err(|error| refuse(error.to_string()))?;
        records.push(TerrainFileRecord {
            terrain: arg.kind.name().to_owned(),
            file: name.to_owned(),
            surface: surface.name().to_owned(),
            sha256: surface.sha256().to_owned(),
            accuracy_metres: arg.accuracy_metres,
            frame: frame.name().to_owned(),
            crs: surface.crs().map(str::to_owned),
            points: surface.point_count(),
            triangles: surface.triangle_count(),
        });
        surfaces.push(surface);
    }
    let terrain = TinTerrain::try_new(surfaces).map_err(|error| error.to_string())?;
    Ok(Terrains {
        service: Some(TerrainServiceHandle::new(Arc::new(terrain))),
        files: records,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terrain_argument_names_its_ground_accuracy_file_and_surface() {
        assert_eq!(
            terrain_arg("original@0.05=dgm.xml"),
            Ok(TerrainArg {
                kind: TerrainKind::Original,
                accuracy_metres: 0.05,
                path: PathBuf::from("dgm.xml"),
                surface: None,
            })
        );
        assert_eq!(
            terrain_arg("Finished@0.1m=dir/site.xml#Design"),
            Ok(TerrainArg {
                kind: TerrainKind::Finished,
                accuracy_metres: 0.1,
                path: PathBuf::from("dir/site.xml"),
                surface: Some("Design".into()),
            })
        );
        for refused in [
            "original=dgm.xml",
            "existing@0.1=dgm.xml",
            "original@=dgm.xml",
            "original@-1=dgm.xml",
            "original@NaN=dgm.xml",
            "original@0.1=",
        ] {
            assert!(terrain_arg(refused).is_err(), "{refused}");
        }
    }
}
