//! What a proven absence measures beside its proof (#391): the obstacles
//! taking the space's floor, sure and possible, the free floor's area, and
//! the largest shape of the declared proportions the search can place,
//! with where it stands.
//!
//! Every value is a certified bracket: the free floor's area as the
//! free-space service brackets it, and the largest shape between a size
//! the search places (a witness, its centre reported) and one it proves
//! absent. Witnesses are asked with every obstacle and swing the
//! selections may hold, absences with the sure ones only, as the decision
//! asks them, and neither with an entrance path: this is the floor as it
//! is, not the part a path reaches.

use std::collections::BTreeSet;

use axioval_engine::{
    BoxClearance, ClearancePlacementEvidence, CylinderClearance, ElevationBand,
    FreeSpaceServiceHandle, PlacementDomain, PlacementOutcome, PlacementRequest, PlacementShape,
    RuleContext, SupportedPlacement, SweptDoor,
};
use axioval_ir::{Evidence, Object, ObjectId};

use crate::door_swing::Swings;
#[cfg(feature = "parity-reference")]
use crate::level_spacing::{metres, shown};
#[cfg(feature = "parity-reference")]
use std::fmt::Write as _;

/// The bisection of the largest shape stops once its bracket is this
/// narrow, in metres of the shape's larger side.
const PRECISION_METRES: f64 = 0.005;

/// The most searches one bisection asks, whatever the bracket.
const MOST_STEPS: usize = 24;

/// What a proven absence measured of the floor; every part empty where
/// the service could not measure it.
#[derive(Default)]
pub(crate) struct Measured {
    /// The obstacles and swept doors surely taking floor, sorted.
    pub(crate) obstacles: Vec<ObjectId>,
    /// Those that may take floor, sorted.
    pub(crate) possible: Vec<ObjectId>,
    /// The free floor's area, bracketed, in square metres.
    pub(crate) area: Option<(f64, f64)>,
    /// The largest shape of the declared proportions.
    pub(crate) largest: Option<Largest>,
    /// What the values rest on.
    pub(crate) evidence: Vec<Evidence>,
}

/// The largest shape of the declared proportions, bracketed: each size
/// (a circle's diameter; a rectangle's width and length) from a placed
/// shape up to one proven absent.
pub(crate) struct Largest {
    pub(crate) sizes: Vec<(f64, f64)>,
    /// The placed shape's centre, in model coordinates; `None` where no
    /// smaller shape was placed.
    pub(crate) centre: Option<[f64; 3]>,
}

/// One measurement's setting.
pub(crate) struct Setting<'a> {
    pub(crate) service: &'a FreeSpaceServiceHandle,
    pub(crate) space: &'a Object,
    pub(crate) merged: &'a [ObjectId],
    pub(crate) shape: &'a PlacementShape,
    pub(crate) band: Option<ElevationBand>,
    /// The obstacles surely selected, and those the selection cannot
    /// decide.
    pub(crate) sure: &'a [ObjectId],
    pub(crate) maybe: &'a [ObjectId],
    pub(crate) swings: &'a Swings,
    /// Whether the declared shape is proven absent without an entrance
    /// path (the decision's proof, or its search without a path).
    pub(crate) absent: bool,
    /// Whether the shape fits where no path reaches it: its size is no
    /// bracket then.
    pub(crate) unreached: bool,
}

/// Measures what `setting`'s proven absence leaves of the floor.
pub(crate) fn measure(context: &RuleContext<'_>, setting: &Setting<'_>) -> Measured {
    let mut measured = floor(setting);
    if setting.absent && !setting.unreached {
        measured.largest = largest(context, setting, &mut measured.evidence);
    }
    measured
}

/// A search request without an entrance path.
fn request(
    setting: &Setting<'_>,
    shape: PlacementShape,
    candidates: &[ObjectId],
    swept: Vec<SweptDoor>,
) -> Option<PlacementRequest> {
    // As the decision asks: a merged search spans several floors, so no
    // single support holds its base.
    let domain = if setting.merged.is_empty() {
        PlacementDomain::Supported(SupportedPlacement::try_new(setting.space.id.clone(), 0.0).ok()?)
    } else {
        PlacementDomain::Unconstrained
    };
    let mut request = PlacementRequest::new_in_domain(
        setting.space.id.clone(),
        shape,
        candidates.to_vec(),
        domain,
    )
    .ok()?
    .with_merged_scopes(setting.merged.to_vec())
    .ok()?;
    if let Some(band) = setting.band {
        request = request.with_band(band);
    }
    if !swept.is_empty() {
        request = request.with_swept_doors(swept).ok()?;
    }
    Some(request)
}

/// Every obstacle and swing the selections may hold.
fn wide(setting: &Setting<'_>) -> (Vec<ObjectId>, Vec<SweptDoor>) {
    let mut candidates = setting.sure.to_vec();
    candidates.extend(setting.maybe.iter().cloned());
    let mut swept = setting.swings.sure.clone();
    swept.extend(setting.swings.maybe.iter().cloned());
    (candidates, swept)
}

/// Whether the selections leave nothing undecided: the wide request is
/// the narrow one.
fn decided(setting: &Setting<'_>) -> bool {
    setting.maybe.is_empty() && setting.swings.maybe.is_empty()
}

/// The free floor: obstacles from the wide request, sure only where the
/// selection is; the area from below with everything that may take floor,
/// from above with what surely does.
fn floor(setting: &Setting<'_>) -> Measured {
    let mut measured = Measured::default();
    let (candidates, swept) = wide(setting);
    let Some(wide) = request(setting, setting.shape.clone(), &candidates, swept)
        .and_then(|request| setting.service.measure_free_floor(&request).ok())
    else {
        return measured;
    };
    let selected: BTreeSet<&ObjectId> = setting
        .sure
        .iter()
        .chain(setting.swings.sure.iter().map(SweptDoor::door))
        .collect();
    for object in wide.obstacles() {
        if selected.contains(object) {
            measured.obstacles.push(object.clone());
        } else {
            measured.possible.push(object.clone());
        }
    }
    measured.possible.extend(wide.possible().iter().cloned());
    measured.possible.sort();
    measured.possible.dedup();
    measured.evidence.push(wide.evidence().clone());
    let narrow = if decided(setting) {
        Some(wide.clone())
    } else {
        request(
            setting,
            setting.shape.clone(),
            setting.sure,
            setting.swings.sure.clone(),
        )
        .and_then(|request| setting.service.measure_free_floor(&request).ok())
    };
    if let Some(narrow) = narrow {
        // A door whose leaves are unknown may sweep any of the floor.
        let lower = if setting.swings.unknown.is_empty() {
            wide.area().lower_square_metres()
        } else {
            0.0
        };
        let upper = narrow.area().upper_square_metres();
        if lower <= upper {
            measured.area = Some((lower, upper));
            measured.evidence.push(narrow.evidence().clone());
        }
    }
    measured
}

/// The declared shape's larger side: a circle's diameter.
fn main_size(shape: &PlacementShape) -> f64 {
    match shape {
        PlacementShape::Cylinder(circle) => 2.0 * circle.radius_metres(),
        PlacementShape::Box { shape, .. } => shape.width_metres().max(shape.depth_metres()),
    }
}

/// The declared shape scaled to `size` on its larger side, and its sizes
/// as asked.
fn scaled(shape: &PlacementShape, size: f64) -> Option<(PlacementShape, Vec<f64>)> {
    match shape {
        PlacementShape::Cylinder(circle) => {
            let radius = size / 2.0;
            let scaled = CylinderClearance::try_new(radius, circle.height_metres()).ok()?;
            Some((PlacementShape::Cylinder(scaled), vec![2.0 * radius]))
        }
        PlacementShape::Box { shape, orientation } => {
            let k = size / main_size_of(shape);
            let (width, length) = (shape.width_metres() * k, shape.depth_metres() * k);
            let scaled = BoxClearance::try_new(width, length, shape.height_metres()).ok()?;
            Some((
                PlacementShape::Box {
                    shape: scaled,
                    orientation: orientation.clone(),
                },
                vec![width, length],
            ))
        }
    }
}

fn main_size_of(shape: &BoxClearance) -> f64 {
    shape.width_metres().max(shape.depth_metres())
}

/// The declared shape's sizes.
fn sizes(shape: &PlacementShape) -> Vec<f64> {
    match shape {
        PlacementShape::Cylinder(circle) => vec![2.0 * circle.radius_metres()],
        PlacementShape::Box { shape, .. } => vec![shape.width_metres(), shape.depth_metres()],
    }
}

/// The largest shape of the declared proportions, by two bisections of
/// its larger side on whole millimetres: one raising the lower end by
/// sizes placed (witnesses, asked with every obstacle and swing the
/// selections may hold, standing only where no door's leaves are
/// unknown), one lowering the upper end by sizes proven absent (asked with
/// the sure ones). A size neither placed nor proven absent (a fit by
/// contact, within the search's margins) bounds each search from its
/// side, so the two close in on it from both. Only a placement and a
/// proof of absence end the bracket; `None` where neither was found.
fn largest(
    context: &RuleContext<'_>,
    setting: &Setting<'_>,
    evidence: &mut Vec<Evidence>,
) -> Option<Largest> {
    let full = main_size(setting.shape);
    // Witnesses are sought between `placing`'s ends (the largest placed,
    // the least not placed), proofs between `proving`'s (the largest not
    // proven absent, the least proven).
    let mut placing = (0.0_f64, full);
    let mut proving = (0.0_f64, full);
    let mut placed: Option<(ClearancePlacementEvidence, Vec<f64>)> = None;
    let mut absent: Option<(Evidence, Vec<f64>)> = None;
    let (candidates, swept) = wide(setting);
    let stands = setting.swings.unknown.is_empty();
    let decided = decided(setting);
    let midpoint = |(low, high): (f64, f64)| {
        let mid = (f64::midpoint(low, high) * 1000.0).round() / 1000.0;
        (high - low > PRECISION_METRES && mid > low && mid < high).then_some(mid)
    };
    let mut turn = false;
    for _ in 0..MOST_STEPS {
        // Each open side in turn; one search serves both where the
        // selections leave nothing undecided.
        let (witness, mid) = match (midpoint(placing), midpoint(proving)) {
            (None, None) => break,
            (Some(mid), None) => (true, mid),
            (None, Some(mid)) => (false, mid),
            (Some(placing), Some(proving)) => {
                turn = !turn;
                if turn {
                    (true, placing)
                } else {
                    (false, proving)
                }
            }
        };
        let (wide, narrow) = (witness || decided, !witness || decided);
        let Some((shape, asked)) = scaled(setting.shape, mid) else {
            break;
        };
        let asking = if wide {
            request(setting, shape, &candidates, swept.clone())
        } else {
            request(setting, shape, setting.sure, setting.swings.sure.clone())
        };
        let Some(asking) = asking else {
            break;
        };
        match setting.service.find_placement(&asking) {
            Ok(PlacementOutcome::Found(found)) if wide && stands => {
                placing.0 = mid;
                placed = Some((found, asked));
            }
            Ok(PlacementOutcome::NoPlacement(proof)) if narrow => {
                proving.1 = mid;
                absent = Some((proof.evidence().clone(), asked));
            }
            // Neither placed nor proven absent here.
            _ => {
                if wide {
                    placing.1 = mid;
                }
                if narrow {
                    proving.0 = mid;
                }
            }
        }
        // Each search is bounded by what the other established.
        proving.0 = proving.0.max(placing.0);
        placing.1 = placing.1.min(proving.1);
    }
    if placed.is_none() && absent.is_none() {
        return None;
    }
    let lows = match &placed {
        Some((_, asked)) => asked.clone(),
        None => vec![0.0; sizes(setting.shape).len()],
    };
    let highs = match &absent {
        Some((_, asked)) => asked.clone(),
        None => sizes(setting.shape),
    };
    let origin = crate::support::local_origin(context, &setting.space.id.source);
    let centre = placed
        .as_ref()
        .map(|(found, _)| origin.point_to_model(found.frame().origin().coordinates_metres()));
    if let Some((found, _)) = &placed {
        evidence.push(found.evidence().clone());
    }
    if let Some((cited, _)) = absent {
        evidence.push(cited);
    }
    Some(Largest {
        sizes: lows.into_iter().zip(highs).collect(),
        centre,
    })
}

/// The words a finding adds for `measured`, as the template renders them:
/// the reference's wording, which the template is held to.
#[cfg(feature = "parity-reference")]
pub(crate) fn words(shape: &PlacementShape, measured: &Measured, unreached: bool) -> String {
    let mut words = String::new();
    if let Some(largest) = &measured.largest {
        let (noun, size) = match shape {
            PlacementShape::Cylinder(_) => (
                "circle",
                format!("{} across", shown(largest.sizes[0].0, largest.sizes[0].1)),
            ),
            PlacementShape::Box { .. } => (
                "rectangle of its proportions",
                format!(
                    "{} by {}",
                    shown(largest.sizes[0].0, largest.sizes[0].1),
                    shown(largest.sizes[1].0, largest.sizes[1].1)
                ),
            ),
        };
        let _ = write!(words, ": the largest free {noun} is {size}");
        if let Some([x, y, z]) = largest.centre {
            let _ = write!(
                words,
                ", centred at ({}, {}, {})",
                metres(x),
                metres(y),
                metres(z)
            );
        }
        let required: Vec<String> = sizes(shape).into_iter().map(metres).collect();
        let _ = write!(words, "; required {}", required.join(" by "));
    }
    if let Some((lower, upper)) = measured.area {
        let separator = if measured.largest.is_none() && !unreached {
            ": "
        } else {
            "; "
        };
        let (low, high) = (area(lower), area(upper));
        let shown = if low == high {
            low
        } else {
            format!("between {low} and {high}")
        };
        let _ = write!(words, "{separator}the free floor is {shown} m²");
    }
    if !measured.possible.is_empty() {
        let names: Vec<String> = measured.possible.iter().map(ToString::to_string).collect();
        let separator = if words.is_empty() && !unreached {
            ": "
        } else {
            "; "
        };
        let _ = write!(
            words,
            "{separator}perhaps also taken by {}",
            names.join(", ")
        );
    }
    words
}

/// An area as the templates show one.
#[cfg(feature = "parity-reference")]
fn area(value: f64) -> String {
    format!("{}", (value * 1e4).round() / 1e4)
}
