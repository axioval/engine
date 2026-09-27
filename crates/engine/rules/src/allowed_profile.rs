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
/// `XDim`, an I-section's overall width, a channel's flange width), `depth`
/// (`YDim`, the overall depth), `web_thickness`, `flange_thickness`,
/// `thickness` (an angle's leg, a centre-line profile's), `wall_thickness`
/// (hollow sections, cold-formed channels), `radius`, `girth` and
/// `fillet_radius`. A row stating a dimension the family does not have does
/// not fit it; one stating a dimension the source leaves unset does not fit
/// either, since the schema default is never assumed.
///
/// Values are exact up to binary rounding, which widens every tolerance by a
/// few units in the last place. A row whose dimension the source refuses is
/// undecided: the object is not evaluated unless another row fits.
pub struct AllowedProfile;

/// The dimension columns, in the order a finding lists them.
const DIMENSIONS: [&str; 9] = [
    "width",
    "depth",
    "web_thickness",
    "flange_thickness",
    "thickness",
    "wall_thickness",
    "radius",
    "girth",
    "fillet_radius",
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
    TableColumn::optional("tolerance", ColumnKind::Quantity),
];

/// The body-set parameter a dimension column reads for a profile family.
fn parameter(family: &str, dimension: &str) -> Option<&'static str> {
    Some(match (family, dimension) {
        ("rectangle" | "rounded-rectangle" | "rectangle-hollow", "width") => "XDim",
        ("rectangle" | "rounded-rectangle" | "rectangle-hollow" | "trapezium", "depth") => "YDim",
        ("rounded-rectangle", "fillet_radius") => "RoundingRadius",
        ("rectangle-hollow" | "circle-hollow" | "c-shape", "wall_thickness") => "WallThickness",
        ("rectangle-hollow", "fillet_radius") => "InnerFilletRadius",
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
    dimensions: Vec<(&'static str, f64)>,
    tolerance: f64,
}

struct Config {
    rows: Vec<Allowed>,
}

fn length_cell(value: f64, unit: &str, what: &str) -> Result<f64, Unavailable> {
    match si_quantity(value, unit)? {
        (value, QuantityDimension::Length) if value >= 0.0 => Ok(value),
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
            for dimension in DIMENSIONS {
                if let Some((value, unit)) = row.quantity(dimension)? {
                    dimensions.push((
                        dimension,
                        length_cell(value, unit, &format!("row {number}'s `{dimension}`"))?,
                    ));
                }
            }
            let tolerance = match row.quantity("tolerance")? {
                Some((value, unit)) => {
                    length_cell(value, unit, &format!("row {number}'s `tolerance`"))?
                }
                None => tolerance,
            };
            rows.push(Allowed {
                number,
                type_pattern,
                type_text: type_text.to_owned(),
                name_pattern,
                dimensions,
                tolerance,
            });
        }
        Ok(Self { rows })
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
struct Profile {
    family: String,
    name: Option<String>,
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
    let wrong = |body: &BodyFacts<'_>, message: String| {
        Ok(Err(finding(
            rule,
            &object.id,
            format!("wrong geometry: {message}; a single swept profile is required"),
            body.evidence().to_vec(),
            vec![],
        )))
    };
    let Some(count) = body.integer("Count")? else {
        return wrong(body, "the object has no body".into());
    };
    if count != 1 {
        return wrong(body, format!("the body has {count} items"));
    }
    let Some(family) = body.text("Profile.Type")? else {
        let kind = body
            .text("Kind")?
            .unwrap_or_else(|| "of an unstated kind".into());
        return wrong(body, format!("the body is a `{kind}`, not a swept profile"));
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
        let Some(parameter) = parameter(&profile.family, dimension) else {
            reasons.push(format!("a `{}` has no {dimension}", profile.family));
            excess = f64::INFINITY;
            continue;
        };
        let actual = match body.length(&format!("{}{parameter}", profile.prefix)) {
            Ok(Some(actual)) => actual,
            Ok(None) => {
                reasons.push(format!("states no {dimension}"));
                excess = f64::INFINITY;
                continue;
            }
            Err(unavailable) => return Some(Fit::Undecided(unavailable)),
        };
        let allowed = row.tolerance + rounding_slack(&[actual, nominal, row.tolerance]);
        let off = (actual - nominal).abs() - allowed;
        if off > 0.0 {
            excess += off;
            let within = if row.tolerance > 0.0 {
                format!(" within {}", metres(row.tolerance))
            } else {
                String::new()
            };
            reasons.push(format!(
                "{dimension} {}, allowed {}{within}",
                metres(actual),
                metres(nominal)
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
        None if ARBITRARY_FAMILIES.contains(&profile.family.as_str()) => format!(
            "arbitrary profile {}: no allowed profile is of its type",
            profile.describe()
        ),
        None => format!("profile {} is not of an allowed type", profile.describe()),
    };
    Ok(Some(finding(
        rule,
        &object.id,
        message,
        body.into_evidence(),
        vec![],
    )))
}
