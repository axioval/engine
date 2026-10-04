//! `allowed-profile`: each member's cross section is one of a table of
//! allowed profiles, by type, name and dimensions within a tolerance.

use std::fmt::Write as _;

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::{Finding, Object, QuantityDimension};

use crate::body_extent::rounding_slack;
use crate::body_facts::{ARBITRARY_FAMILIES, BodyFacts};
use crate::level_spacing::metres;
use crate::selection::select_objects;
use crate::support::table::{RowTest, TextPattern};
use crate::support::{Parameters, Unavailable, finding, invalid, si_quantity};

/// Requires each selected object's body to be one swept solid whose profile
/// is a row of the `profiles` table: its type, its name where the row
/// states one, and every dimension the row states within the tolerance.
///
/// The profile is read from the reserved body set (`axioval:body`), so the
/// source must state how the body is modelled. Three results are told
/// apart: a body that is no single swept profile (no body, several items,
/// or a boundary representation, a tessellation, …) is wrong geometry; a
/// profile of an arbitrary outline (`arbitrary-closed`, `composite`, …)
/// that no row allows is an arbitrary profile; a parameterised profile no
/// row fits is not an allowed profile, and the finding names the nearest
/// row of its type and how far each dimension is off. A mirrored profile is
/// judged by its parent, whose dimensions it keeps; a derived one is not
/// evaluated, since its operator may scale the parent.
///
/// Dimensions are named alike for every family: `width` (a rectangle's
/// `XDim`, an I-section's overall width, a channel's flange width, an
/// asymmetric I-section's bottom flange width, a trapezium's bottom width),
/// `depth` (`YDim`, the overall depth), `web_thickness`, `flange_thickness`,
/// `thickness` (an angle's leg, a centre-line profile's), `wall_thickness`
/// (hollow sections, cold-formed channels), `radius`, `girth`,
/// `fillet_radius`, the ellipse's `semi_axis_1` and `semi_axis_2`, the top
/// of a trapezium or an asymmetric I-section (`top_width`, `top_offset`,
/// `top_flange_thickness`, `top_fillet_radius`, `top_edge_radius`), edge
/// radii (`edge_radius`, `web_edge_radius`, `outer_fillet_radius`), and the
/// slopes, plane angles judged within `angle_tolerance` (`flange_slope`,
/// `top_flange_slope`, `leg_slope`, `web_slope`). A row stating a dimension
/// the family does not have does not fit it; one stating a dimension the
/// source leaves unset does not fit either, since the schema default is
/// never assumed.
///
/// With `match: rows` (the default) a profile must fit one whole row. With
/// `match: per_dimension` the rows of its type (and name) list the allowed
/// values of each dimension apart: a width from one row with a depth from
/// another fits, so five widths by six depths take six rows, not thirty.
///
/// Values are exact up to binary rounding, which widens every tolerance by a
/// few units in the last place. A row whose dimension the source refuses is
/// undecided: the object is not evaluated unless another row fits.
pub struct AllowedProfile;

mod measured;

pub(crate) use measured::ProfileMeasures;

/// One dimension column: a length, or a plane angle; a length that may be
/// negative is an offset.
struct Dimension {
    column: &'static str,
    angle: bool,
    signed: bool,
}

const fn length(column: &'static str) -> Dimension {
    Dimension {
        column,
        angle: false,
        signed: false,
    }
}

const fn angle(column: &'static str) -> Dimension {
    Dimension {
        column,
        angle: true,
        signed: false,
    }
}

/// The dimension columns, in the order a finding lists them.
const DIMENSIONS: [Dimension; 23] = [
    length("width"),
    length("depth"),
    length("web_thickness"),
    length("flange_thickness"),
    length("thickness"),
    length("wall_thickness"),
    length("radius"),
    length("girth"),
    length("fillet_radius"),
    length("semi_axis_1"),
    length("semi_axis_2"),
    length("top_width"),
    Dimension {
        column: "top_offset",
        angle: false,
        signed: true,
    },
    length("top_flange_thickness"),
    length("top_fillet_radius"),
    length("edge_radius"),
    length("top_edge_radius"),
    length("web_edge_radius"),
    length("outer_fillet_radius"),
    angle("flange_slope"),
    angle("top_flange_slope"),
    angle("leg_slope"),
    angle("web_slope"),
];

const COLUMNS: &[TableColumn] = &[
    TableColumn::required("type", ColumnKind::TextPattern),
    TableColumn::optional("name", ColumnKind::TextPattern),
    TableColumn::optional("width", ColumnKind::Quantity),
    TableColumn::optional("depth", ColumnKind::Quantity),
    TableColumn::optional("web_thickness", ColumnKind::Quantity),
    TableColumn::optional("flange_thickness", ColumnKind::Quantity),
    TableColumn::optional("thickness", ColumnKind::Quantity),
    TableColumn::optional("wall_thickness", ColumnKind::Quantity),
    TableColumn::optional("radius", ColumnKind::Quantity),
    TableColumn::optional("girth", ColumnKind::Quantity),
    TableColumn::optional("fillet_radius", ColumnKind::Quantity),
    TableColumn::optional("semi_axis_1", ColumnKind::Quantity),
    TableColumn::optional("semi_axis_2", ColumnKind::Quantity),
    TableColumn::optional("top_width", ColumnKind::Quantity),
    TableColumn::optional("top_offset", ColumnKind::Quantity),
    TableColumn::optional("top_flange_thickness", ColumnKind::Quantity),
    TableColumn::optional("top_fillet_radius", ColumnKind::Quantity),
    TableColumn::optional("edge_radius", ColumnKind::Quantity),
    TableColumn::optional("top_edge_radius", ColumnKind::Quantity),
    TableColumn::optional("web_edge_radius", ColumnKind::Quantity),
    TableColumn::optional("outer_fillet_radius", ColumnKind::Quantity),
    TableColumn::optional("flange_slope", ColumnKind::Quantity),
    TableColumn::optional("top_flange_slope", ColumnKind::Quantity),
    TableColumn::optional("leg_slope", ColumnKind::Quantity),
    TableColumn::optional("web_slope", ColumnKind::Quantity),
    TableColumn::optional("tolerance", ColumnKind::Quantity),
    TableColumn::optional("angle_tolerance", ColumnKind::Quantity),
];

/// The body-set parameter a dimension column reads for a profile family.
fn parameter(family: &str, dimension: &str) -> Option<&'static str> {
    Some(match (family, dimension) {
        ("rectangle" | "rounded-rectangle" | "rectangle-hollow", "width") => "XDim",
        ("rectangle" | "rounded-rectangle" | "rectangle-hollow" | "trapezium", "depth") => "YDim",
        ("rounded-rectangle", "fillet_radius") => "RoundingRadius",
        ("rectangle-hollow" | "circle-hollow" | "c-shape", "wall_thickness") => "WallThickness",
        ("rectangle-hollow", "fillet_radius") => "InnerFilletRadius",
        ("rectangle-hollow", "outer_fillet_radius") => "OuterFilletRadius",
        ("ellipse", "semi_axis_1") => "SemiAxis1",
        ("ellipse", "semi_axis_2") => "SemiAxis2",
        ("trapezium", "width") => "BottomXDim",
        ("trapezium", "top_width") => "TopXDim",
        ("trapezium", "top_offset") => "TopXOffset",
        ("asymmetric-i-shape", "width") => "BottomFlangeWidth",
        ("asymmetric-i-shape", "flange_thickness") => "BottomFlangeThickness",
        ("asymmetric-i-shape", "fillet_radius") => "BottomFlangeFilletRadius",
        ("asymmetric-i-shape", "edge_radius") => "BottomFlangeEdgeRadius",
        ("asymmetric-i-shape", "flange_slope") => "BottomFlangeSlope",
        ("asymmetric-i-shape", "top_width") => "TopFlangeWidth",
        ("asymmetric-i-shape", "top_flange_thickness") => "TopFlangeThickness",
        ("asymmetric-i-shape", "top_fillet_radius") => "TopFlangeFilletRadius",
        ("asymmetric-i-shape", "top_edge_radius") => "TopFlangeEdgeRadius",
        ("asymmetric-i-shape", "top_flange_slope") => "TopFlangeSlope",
        ("i-shape" | "t-shape", "edge_radius") => "FlangeEdgeRadius",
        ("l-shape" | "u-shape" | "z-shape", "edge_radius") => "EdgeRadius",
        ("t-shape", "web_edge_radius") => "WebEdgeRadius",
        ("i-shape" | "t-shape" | "u-shape", "flange_slope") => "FlangeSlope",
        ("l-shape", "leg_slope") => "LegSlope",
        ("t-shape", "web_slope") => "WebSlope",
        ("circle" | "circle-hollow", "radius") => "Radius",
        ("i-shape", "width") => "OverallWidth",
        ("i-shape" | "asymmetric-i-shape", "depth") => "OverallDepth",
        ("i-shape" | "asymmetric-i-shape" | "t-shape" | "u-shape" | "z-shape", "web_thickness") => {
            "WebThickness"
        }
        ("i-shape" | "t-shape" | "u-shape" | "z-shape", "flange_thickness") => "FlangeThickness",
        ("i-shape" | "l-shape" | "t-shape" | "u-shape" | "z-shape", "fillet_radius") => {
            "FilletRadius"
        }
        ("l-shape" | "t-shape" | "u-shape" | "c-shape" | "z-shape", "depth") => "Depth",
        ("l-shape" | "c-shape", "width") => "Width",
        ("t-shape" | "u-shape" | "z-shape", "width") => "FlangeWidth",
        ("l-shape" | "center-line", "thickness") => "Thickness",
        ("c-shape", "girth") => "Girth",
        ("c-shape", "fillet_radius") => "InternalFilletRadius",
        _ => return None,
    })
}

/// One row of the table, patterns compiled and quantities in metres.
struct Allowed {
    /// One-based position, as a reviewer counts rows.
    number: usize,
    type_pattern: TextPattern,
    type_text: String,
    name_pattern: Option<(TextPattern, String)>,
    dimensions: Vec<(&'static Dimension, f64)>,
    tolerance: f64,
    angle_tolerance: f64,
}

impl Allowed {
    /// How far `actual` lies beyond this row's tolerance of `nominal`; at
    /// most zero when it fits.
    fn excess(&self, dimension: &Dimension, actual: f64, nominal: f64) -> f64 {
        let tolerance = if dimension.angle {
            self.angle_tolerance
        } else {
            self.tolerance
        };
        let allowed = tolerance + rounding_slack(&[actual, nominal, tolerance]);
        (actual - nominal).abs() - allowed
    }

    /// The row's value of `dimension`, as a finding lists it.
    fn allowed(&self, dimension: &Dimension, nominal: f64) -> String {
        let tolerance = if dimension.angle {
            self.angle_tolerance
        } else {
            self.tolerance
        };
        let within = if tolerance > 0.0 {
            format!(" within {}", shown(dimension, tolerance))
        } else {
            String::new()
        };
        format!("{}{within}", shown(dimension, nominal))
    }
}

/// A dimension's value as a reviewer reads it: metres, or degrees.
fn shown(dimension: &Dimension, value: f64) -> String {
    if dimension.angle {
        format!("{}°", (value.to_degrees() * 1e6).round() / 1e6)
    } else {
        metres(value)
    }
}

/// Whether a profile fits one whole row, or each dimension any row's value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Match {
    Rows,
    PerDimension,
}

struct Config {
    rows: Vec<Allowed>,
    matching: Match,
}

fn length_cell(value: f64, unit: &str, what: &str) -> Result<f64, Unavailable> {
    match si_quantity(value, unit)? {
        (value, QuantityDimension::Length) if value >= 0.0 => Ok(value),
        (_, QuantityDimension::Length) => Err(invalid(format!("{what} is negative"))),
        _ => Err(invalid(format!("{what} is not a length"))),
    }
}

fn angle_cell(value: f64, unit: &str, what: &str) -> Result<f64, Unavailable> {
    match si_quantity(value, unit)? {
        (value, QuantityDimension::PlaneAngle) if value >= 0.0 => Ok(value),
        (_, QuantityDimension::PlaneAngle) => Err(invalid(format!("{what} is negative"))),
        _ => Err(invalid(format!("{what} is not a plane angle"))),
    }
}

/// A dimension cell in canonical SI: a length (an offset may be negative)
/// or a non-negative angle.
fn dimension_cell(
    dimension: &Dimension,
    value: f64,
    unit: &str,
    what: &str,
) -> Result<f64, Unavailable> {
    if dimension.angle {
        return angle_cell(value, unit, what);
    }
    match si_quantity(value, unit)? {
        (value, QuantityDimension::Length) if dimension.signed || value >= 0.0 => Ok(value),
        (_, QuantityDimension::Length) => Err(invalid(format!("{what} is negative"))),
        _ => Err(invalid(format!("{what} is not a length"))),
    }
}

impl Config {
    fn parse(rule: &CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(false);
        let tolerance = match parameters.quantity("tolerance")? {
            None => 0.0,
            Some((value, QuantityDimension::Length)) if value >= 0.0 => value,
            Some(_) => return Err(invalid("`tolerance` is not a non-negative length")),
        };
        let angle_tolerance = match parameters.quantity("angle_tolerance")? {
            None => 0.0,
            Some((value, QuantityDimension::PlaneAngle)) if value >= 0.0 => value,
            Some(_) => {
                return Err(invalid(
                    "`angle_tolerance` is not a non-negative plane angle",
                ));
            }
        };
        let matching = match parameters.string("match")? {
            None | Some("rows") => Match::Rows,
            Some("per_dimension") => Match::PerDimension,
            Some(other) => {
                return Err(invalid(format!(
                    "`match` is `{other}`, not `rows` or `per_dimension`"
                )));
            }
        };
        let table = parameters
            .table("profiles")?
            .ok_or_else(|| invalid("parameter `profiles` is required"))?;
        if table.is_empty() {
            return Err(invalid("`profiles` has no rows"));
        }
        let mut rows = Vec::with_capacity(table.len());
        for (index, row) in table.into_iter().enumerate() {
            let number = index + 1;
            let type_text = row
                .text("type")?
                .ok_or_else(|| invalid(format!("row {number} states no `type`")))?;
            let type_pattern = TextPattern::new(type_text, case_sensitive).map_err(invalid)?;
            let name_pattern = row
                .text("name")?
                .map(|name| {
                    TextPattern::new(name, case_sensitive)
                        .map(|pattern| (pattern, name.to_owned()))
                        .map_err(invalid)
                })
                .transpose()?;
            let mut dimensions = Vec::new();
            for dimension in &DIMENSIONS {
                if let Some((value, unit)) = row.quantity(dimension.column)? {
                    dimensions.push((
                        dimension,
                        dimension_cell(
                            dimension,
                            value,
                            unit,
                            &format!("row {number}'s `{}`", dimension.column),
                        )?,
                    ));
                }
            }
            let tolerance = match row.quantity("tolerance")? {
                Some((value, unit)) => {
                    length_cell(value, unit, &format!("row {number}'s `tolerance`"))?
                }
                None => tolerance,
            };
            let angle_tolerance = match row.quantity("angle_tolerance")? {
                Some((value, unit)) => {
                    angle_cell(value, unit, &format!("row {number}'s `angle_tolerance`"))?
                }
                None => angle_tolerance,
            };
            rows.push(Allowed {
                number,
                type_pattern,
                type_text: type_text.to_owned(),
                name_pattern,
                dimensions,
                tolerance,
                angle_tolerance,
            });
        }
        Ok(Self { rows, matching })
    }
}

impl RuleCapability for AllowedProfile {
    fn id(&self) -> &'static str {
        "axioval:capability.allowed-profile"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("profiles", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::optional("tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("angle_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("match", ParameterType::String),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("allowed-profile: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            match check(context, rule, &config, object) {
                Ok(Some(found)) => evaluation.push_finding(found),
                Ok(None) => {}
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

/// How one row relates to the profile.
enum Fit {
    Fits,
    /// It does not fit; the sum of the excesses beyond the tolerance (infinite
    /// when a dimension cannot be compared at all), whether the name
    /// mismatches, and what is off, for the message.
    Off {
        excess: f64,
        name_off: bool,
        reasons: Vec<String>,
    },
    Undecided(Unavailable),
}

/// The profile as the table reads it: family, name, and where its
/// dimensions are stated in the body set.
pub(crate) struct Profile {
    pub(crate) family: String,
    pub(crate) name: Option<String>,
    /// `Profile.`, or `Profile.Parent.` … for a mirrored profile.
    prefix: String,
    mirrored: bool,
}

impl Profile {
    fn describe(&self) -> String {
        let mut text = format!("`{}`", self.family);
        if let Some(name) = &self.name {
            let _ = write!(text, " `{name}`");
        }
        if self.mirrored {
            text.push_str(" (mirrored)");
        }
        text
    }
}

/// Reads the profile, or the finding or refusal when there is none to judge.
fn profile(
    rule: &CompiledRule,
    object: &Object,
    body: &mut BodyFacts<'_>,
) -> Result<Result<Profile, Finding>, Unavailable> {
    Ok(read_profile(body)?.map_err(|message| {
        finding(
            rule,
            &object.id,
            format!("wrong geometry: {message}; a single swept profile is required"),
            body.evidence().to_vec(),
            vec![],
        )
    }))
}

/// Reads the body's one swept profile, following a mirror to its parent;
/// why the body has none (no body, several items, no swept solid), or a
/// refusal when that cannot be read or a derived profile's operator may
/// scale its parent.
pub(crate) fn read_profile(
    body: &mut BodyFacts<'_>,
) -> Result<Result<Profile, String>, Unavailable> {
    let Some(count) = body.integer("Count")? else {
        return Ok(Err("the object has no body".into()));
    };
    if count != 1 {
        return Ok(Err(format!("the body has {count} items")));
    }
    let Some(family) = body.text("Profile.Type")? else {
        let kind = body
            .text("Kind")?
            .unwrap_or_else(|| "of an unstated kind".into());
        return Ok(Err(format!("the body is a `{kind}`, not a swept profile")));
    };
    let name = body.text("Profile.Name")?;
    let mut profile = Profile {
        family,
        name,
        prefix: "Profile.".into(),
        mirrored: false,
    };
    // A mirror keeps its parent's dimensions; follow it to the parent.
    while profile.family == "mirrored" {
        profile.prefix.push_str("Parent.");
        profile.mirrored = true;
        profile.family = body
            .text(&format!("{}Type", profile.prefix))?
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::IncompleteEvidence,
                    "the mirrored profile states no parent".to_owned(),
                )
            })?;
        if profile.name.is_none() {
            profile.name = body.text(&format!("{}Name", profile.prefix))?;
        }
    }
    if profile.family == "derived" {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            "a derived profile's operator may scale its parent, and the body does not state \
             the operator"
                .into(),
        ));
    }
    Ok(Ok(profile))
}

/// The dimension columns, in the order a finding lists them: each name and
/// whether it is a plane angle (otherwise a length).
pub(crate) fn dimension_columns() -> impl Iterator<Item = (&'static str, bool)> {
    DIMENSIONS
        .iter()
        .map(|dimension| (dimension.column, dimension.angle))
}

/// `profile`'s value of the dimension column `column` in metres or
/// radians: `None` when its family has no such dimension or the source
/// leaves it unset.
pub(crate) fn dimension_value(
    profile: &Profile,
    column: &str,
    body: &mut BodyFacts<'_>,
) -> Result<Option<f64>, Unavailable> {
    let Some(dimension) = DIMENSIONS
        .iter()
        .find(|dimension| dimension.column == column)
    else {
        return Ok(None);
    };
    match measure(profile, dimension, body) {
        Measured::Value(value) => Ok(Some(value)),
        Measured::Missing(_) => Ok(None),
        Measured::Undecided(unavailable) => Err(unavailable),
    }
}

/// A profile's value of one dimension column.
enum Measured {
    Value(f64),
    /// The family has no such dimension, or the source leaves it unset: it
    /// fits no stated value.
    Missing(String),
    Undecided(Unavailable),
}

fn measure(profile: &Profile, dimension: &Dimension, body: &mut BodyFacts<'_>) -> Measured {
    let column = dimension.column;
    let Some(parameter) = parameter(&profile.family, column) else {
        return Measured::Missing(format!("a `{}` has no {column}", profile.family));
    };
    let name = format!("{}{parameter}", profile.prefix);
    let read = if dimension.angle {
        body.angle(&name)
    } else {
        body.length(&name)
    };
    match read {
        Ok(Some(value)) => Measured::Value(value),
        Ok(None) => Measured::Missing(format!("states no {column}")),
        Err(unavailable) => Measured::Undecided(unavailable),
    }
}

/// Whether `row` applies to `profile` by its type, and by its name.
fn named(row: &Allowed, profile: &Profile) -> bool {
    match (&row.name_pattern, &profile.name) {
        (None, _) => true,
        (Some((pattern, _)), Some(name)) => pattern.test(name) != RowTest::NoMatch,
        (Some(_), None) => false,
    }
}

/// Judges a profile under `match: per_dimension`: every dimension the rows
/// of its type and name state must lie within some such row's value. A
/// dimension no value fits is a finding, whatever else is undecided.
fn per_dimension(
    config: &Config,
    profile: &Profile,
    body: &mut BodyFacts<'_>,
) -> Result<Option<String>, Unavailable> {
    let typed: Vec<&Allowed> = config
        .rows
        .iter()
        .filter(|row| row.type_pattern.test(&profile.family) != RowTest::NoMatch)
        .collect();
    if typed.is_empty() {
        return Ok(Some(no_row_of_type(profile)));
    }
    let rows: Vec<&Allowed> = typed
        .iter()
        .copied()
        .filter(|row| named(row, profile))
        .collect();
    if rows.is_empty() {
        let numbers: Vec<String> = typed.iter().map(|row| row.number.to_string()).collect();
        return Ok(Some(format!(
            "profile {} is not an allowed profile: its name fits none of rows {} of its type",
            profile.describe(),
            numbers.join(", ")
        )));
    }
    let mut failures = Vec::new();
    let mut undecided = None;
    for dimension in &DIMENSIONS {
        let values: Vec<(&Allowed, f64)> = rows
            .iter()
            .flat_map(|row| {
                row.dimensions
                    .iter()
                    .filter(|(stated, _)| stated.column == dimension.column)
                    .map(move |&(_, nominal)| (*row, nominal))
            })
            .collect();
        if values.is_empty() {
            continue;
        }
        let actual = match measure(profile, dimension, body) {
            Measured::Value(actual) => actual,
            Measured::Missing(reason) => {
                failures.push(reason);
                continue;
            }
            Measured::Undecided(unavailable) => {
                undecided.get_or_insert(unavailable);
                continue;
            }
        };
        if values
            .iter()
            .any(|(row, nominal)| row.excess(dimension, actual, *nominal) <= 0.0)
        {
            continue;
        }
        let allowed: Vec<String> = values
            .iter()
            .map(|(row, nominal)| {
                format!("{} (row {})", row.allowed(dimension, *nominal), row.number)
            })
            .collect();
        failures.push(format!(
            "{} {} is none of {}",
            dimension.column,
            shown(dimension, actual),
            allowed.join(", ")
        ));
    }
    if !failures.is_empty() {
        return Ok(Some(format!(
            "profile {} is not an allowed profile: {}",
            profile.describe(),
            failures.join("; ")
        )));
    }
    match undecided {
        Some((reason, message)) => Err((
            reason,
            format!(
                "profile {}: a dimension it may fit is undecided: {message}",
                profile.describe()
            ),
        )),
        None => Ok(None),
    }
}

/// The finding for a profile no row names the type of.
fn no_row_of_type(profile: &Profile) -> String {
    if ARBITRARY_FAMILIES.contains(&profile.family.as_str()) {
        format!(
            "arbitrary profile {}: no allowed profile is of its type",
            profile.describe()
        )
    } else {
        format!("profile {} is not of an allowed type", profile.describe())
    }
}

fn fit(row: &Allowed, profile: &Profile, body: &mut BodyFacts<'_>) -> Option<Fit> {
    if row.type_pattern.test(&profile.family) == RowTest::NoMatch {
        return None;
    }
    let mut reasons = Vec::new();
    let mut excess = 0.0;
    let name_off = match (&row.name_pattern, &profile.name) {
        (None, _) => false,
        (Some((pattern, text)), Some(name)) => {
            let off = pattern.test(name) == RowTest::NoMatch;
            if off {
                reasons.push(format!("name is not `{text}`"));
            }
            off
        }
        (Some((_, text)), None) => {
            reasons.push(format!("states no name, `{text}` required"));
            true
        }
    };
    for &(dimension, nominal) in &row.dimensions {
        let actual = match measure(profile, dimension, body) {
            Measured::Value(actual) => actual,
            Measured::Missing(reason) => {
                reasons.push(reason);
                excess = f64::INFINITY;
                continue;
            }
            Measured::Undecided(unavailable) => return Some(Fit::Undecided(unavailable)),
        };
        let off = row.excess(dimension, actual, nominal);
        if off > 0.0 {
            excess += off;
            reasons.push(format!(
                "{} {}, allowed {}",
                dimension.column,
                shown(dimension, actual),
                row.allowed(dimension, nominal)
            ));
        }
    }
    Some(if reasons.is_empty() {
        Fit::Fits
    } else {
        Fit::Off {
            excess,
            name_off,
            reasons,
        }
    })
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    config: &Config,
    object: &Object,
) -> Result<Option<Finding>, Unavailable> {
    let mut body = BodyFacts::of(context, object)?;
    let profile = match profile(rule, object, &mut body)? {
        Ok(profile) => profile,
        Err(found) => return Ok(Some(found)),
    };
    if config.matching == Match::PerDimension {
        return Ok(per_dimension(config, &profile, &mut body)?
            .map(|message| finding(rule, &object.id, message, body.into_evidence(), vec![])));
    }
    let mut undecided = None;
    // The nearest row of the profile's type: least excess, then a matching
    // name, then the first declared. It only words the finding.
    let mut nearest: Option<(f64, bool, &Allowed, Vec<String>)> = None;
    for row in &config.rows {
        match fit(row, &profile, &mut body) {
            None => {}
            Some(Fit::Fits) => return Ok(None),
            Some(Fit::Undecided(unavailable)) => {
                undecided.get_or_insert(unavailable);
            }
            Some(Fit::Off {
                excess,
                name_off,
                reasons,
            }) => {
                let closer = nearest
                    .as_ref()
                    .is_none_or(|(least, least_name, _, _)| match excess.total_cmp(least) {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Equal => !name_off && *least_name,
                        std::cmp::Ordering::Greater => false,
                    });
                if closer {
                    nearest = Some((excess, name_off, row, reasons));
                }
            }
        }
    }
    if let Some((reason, message)) = undecided {
        return Err((
            reason,
            format!(
                "profile {}: a row it may fit is undecided: {message}",
                profile.describe()
            ),
        ));
    }
    let message = match nearest {
        Some((_, _, row, reasons)) => {
            let name = row
                .name_pattern
                .as_ref()
                .map_or_else(String::new, |(_, text)| format!(" `{text}`"));
            format!(
                "profile {} is not an allowed profile; nearest is row {} (`{}`{name}): {}",
                profile.describe(),
                row.number,
                row.type_text,
                reasons.join("; ")
            )
        }
        None => no_row_of_type(&profile),
    };
    Ok(Some(finding(
        rule,
        &object.id,
        message,
        body.into_evidence(),
        vec![],
    )))
}
