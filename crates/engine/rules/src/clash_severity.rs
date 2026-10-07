//! Clash severities by class and size, and what duplicates differ in.
//!
//! A clash rule's findings need not share one severity. `severity_by_class`
//! gives each class its own (a duplicate is usually worse than a touching
//! intersection), and `severity_grades` grades intersections by their size:
//! the smallest extent of the intersection (`grade_by: smallest_extent`) or
//! the volume the bodies share (`grade_by: volume`). An intersection whose
//! measure exceeds a grade's `above` takes the severity of the highest such
//! grade; one exceeding none keeps its class's.
//!
//! Measures are intervals. A measure straddling a grade's bound could fall
//! in more than one grade, and an unmeasured one in any: it takes the most
//! severe severity it may reach, and its message says so, so a clash is
//! never reported milder than it may be.
//!
//! A duplicate's finding also states what the copies differ in: their types,
//! their measured volumes when the intervals are apart, and the quantities
//! `duplicate_quantities` names, read from the source. A quantity that
//! cannot be read is named as unknown, never as the same.

#[cfg(feature = "parity-reference")]
use axioval_engine::LengthInterval;
use axioval_engine::{
    ColumnKind, ParameterDescriptor, ParameterType, ProximityEvidence, RuleContext, TableColumn,
};
use axioval_ir::{Evidence, ObjectId, Severity};

use crate::clash::Class;
use crate::support::table::Row;
use crate::support::{Parameters, PropertyRef, Unavailable, display, invalid, resolve, value_key};

const CLASS_COLUMNS: &[TableColumn] = &[
    TableColumn::required("class", ColumnKind::String),
    TableColumn::required("severity", ColumnKind::String),
];

const GRADE_COLUMNS: &[TableColumn] = &[
    TableColumn::required("above", ColumnKind::Number),
    TableColumn::required("severity", ColumnKind::String),
];

const QUANTITY_COLUMNS: &[TableColumn] = &[
    TableColumn::optional("property_set", ColumnKind::String),
    TableColumn::required("property", ColumnKind::String),
];

/// The severity and duplicate parameters `clash` and `clash-matrix` share.
pub(crate) fn severity_parameters() -> [ParameterDescriptor; 4] {
    [
        ParameterDescriptor::optional("severity_by_class", ParameterType::Table(CLASS_COLUMNS)),
        ParameterDescriptor::optional("grade_by", ParameterType::String),
        ParameterDescriptor::optional("severity_grades", ParameterType::Table(GRADE_COLUMNS)),
        ParameterDescriptor::optional(
            "duplicate_quantities",
            ParameterType::Table(QUANTITY_COLUMNS),
        ),
    ]
}

/// A severity as a rule states it.
pub(crate) fn parse_severity(text: &str) -> Result<Severity, Unavailable> {
    match text {
        "error" => Ok(Severity::Error),
        "warning" => Ok(Severity::Warning),
        "info" => Ok(Severity::Info),
        other => Err(invalid(format!(
            "severity `{other}` is not `error`, `warning` or `info`"
        ))),
    }
}

pub(crate) fn label(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// What intersections are graded by.
#[derive(Clone, Copy)]
pub(crate) enum Measure {
    SmallestExtent,
    Volume,
}

impl Measure {
    #[cfg(feature = "parity-reference")]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::SmallestExtent => "smallest extent",
            Self::Volume => "shared volume",
        }
    }

    /// The measure's interval; `None` when it was not measured.
    #[cfg(feature = "parity-reference")]
    pub(crate) fn interval(self, measured: &ProximityEvidence) -> Option<(f64, f64)> {
        match self {
            Self::SmallestExtent => measured.overlap_extents().map(|extents| {
                let axes: [LengthInterval; 3] = [extents.x(), extents.y(), extents.z()];
                axes.iter()
                    .fold((f64::INFINITY, f64::INFINITY), |(lower, upper), axis| {
                        (
                            lower.min(axis.lower_metres()),
                            upper.min(axis.upper_metres()),
                        )
                    })
            }),
            Self::Volume => measured.intersection_volume().map(|volume| {
                let shared = volume.shared();
                (shared.lower_cubic_metres(), shared.upper_cubic_metres())
            }),
        }
    }

    pub(crate) fn describe(self, (lower, upper): (f64, f64)) -> String {
        let (digits, unit) = match self {
            Self::SmallestExtent => (4, "m"),
            Self::Volume => (6, "m³"),
        };
        if lower.total_cmp(&upper).is_eq() {
            format!("{lower:.digits$} {unit}")
        } else {
            format!("{lower:.digits$} to {upper:.digits$} {unit}")
        }
    }
}

/// A quantity duplicates are compared by.
struct Quantity {
    set: Option<String>,
    name: String,
}

/// The declared severities and duplicate comparisons.
pub(crate) struct Severities {
    #[cfg(feature = "parity-reference")]
    pub(crate) by_class: Vec<(Class, Severity)>,
    /// What intersections are graded by, and the grades by rising bound.
    #[cfg(feature = "parity-reference")]
    pub(crate) grades: Option<(Measure, Vec<(f64, Severity)>)>,
    quantities: Vec<Quantity>,
}

/// Reads the declared severities; none declared keeps the rule's.
pub(crate) fn severities(parameters: &Parameters<'_>) -> Result<Severities, Unavailable> {
    let mut by_class: Vec<(Class, Severity)> = Vec::new();
    for row in parameters.table("severity_by_class")?.unwrap_or_default() {
        let class = match row.text("class")?.unwrap_or_default() {
            "duplicate" => Class::Duplicate,
            "containment" => Class::Containment,
            "intersection" => Class::Intersection,
            "clearance" => Class::Clearance,
            other => {
                return Err(invalid(format!(
                    "`severity_by_class` class `{other}` is not `duplicate`, `containment`, \
                     `intersection` or `clearance`"
                )));
            }
        };
        if by_class.iter().any(|(known, _)| *known == class) {
            return Err(invalid(format!(
                "`severity_by_class` states class `{}` twice",
                class.name()
            )));
        }
        by_class.push((class, row_severity(row)?));
    }

    let measure = parameters
        .string("grade_by")?
        .map(|measure| match measure {
            "smallest_extent" => Ok(Measure::SmallestExtent),
            "volume" => Ok(Measure::Volume),
            other => Err(invalid(format!(
                "`grade_by` `{other}` is not `smallest_extent` or `volume`"
            ))),
        })
        .transpose()?;
    let rows = parameters.table("severity_grades")?.unwrap_or_default();
    let grades = match (measure, rows.is_empty()) {
        (None, true) => None,
        (Some(measure), false) => {
            let mut grades = Vec::with_capacity(rows.len());
            for row in rows {
                let above = row.number("above")?.unwrap_or(-1.0);
                if !(above >= 0.0 && above.is_finite()) {
                    return Err(invalid("a `severity_grades` bound must not be negative"));
                }
                grades.push((above, row_severity(row)?));
            }
            grades.sort_by(|a, b| a.0.total_cmp(&b.0));
            if grades
                .windows(2)
                .any(|pair| pair[0].0.total_cmp(&pair[1].0).is_eq())
            {
                return Err(invalid("`severity_grades` states one bound twice"));
            }
            Some((measure, grades))
        }
        (None, false) => {
            return Err(invalid(
                "`severity_grades` needs `grade_by`: `smallest_extent` or `volume`",
            ));
        }
        (Some(_), true) => {
            return Err(invalid("`grade_by` is declared without `severity_grades`"));
        }
    };

    let quantities = parameters
        .table("duplicate_quantities")?
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            Ok(Quantity {
                set: row.text("property_set")?.map(ToOwned::to_owned),
                name: row
                    .text("property")?
                    .ok_or_else(|| invalid("a `duplicate_quantities` row names no property"))?
                    .to_owned(),
            })
        })
        .collect::<Result<_, Unavailable>>()?;
    // Only the parity reference keeps the classes and grades (the template
    // reads its own); they are read here to refuse a bad declaration.
    #[cfg(not(feature = "parity-reference"))]
    let _ = grades;
    Ok(Severities {
        #[cfg(feature = "parity-reference")]
        by_class,
        #[cfg(feature = "parity-reference")]
        grades,
        quantities,
    })
}

fn row_severity(row: Row<'_>) -> Result<Severity, Unavailable> {
    parse_severity(row.text("severity")?.unwrap_or_default())
}

impl Severities {
    /// What two duplicates differ in, as a message suffix, with the
    /// evidence of every quantity read.
    pub(crate) fn differences(
        &self,
        context: &RuleContext<'_>,
        measured: &ProximityEvidence,
        (subject, counterpart): (&ObjectId, &ObjectId),
    ) -> (String, Vec<Evidence>) {
        let mut differ: Vec<String> = Vec::new();
        let mut unknown: Vec<String> = Vec::new();
        let mut agree: Vec<String> = Vec::new();
        let mut evidence = Vec::new();
        let objects = (
            context.project.object(subject),
            context.project.object(counterpart),
        );
        match objects {
            (Some(a), Some(b)) if a.kind != b.kind => {
                differ.push(format!("type ({} and {})", a.kind, b.kind));
            }
            (Some(_), Some(_)) => agree.push("type".to_owned()),
            _ => unknown.push("type".to_owned()),
        }
        match measured.intersection_volume() {
            Some(volume) => {
                let (a, b) = (volume.subject(), volume.counterpart());
                if a.upper_cubic_metres() < b.lower_cubic_metres()
                    || b.upper_cubic_metres() < a.lower_cubic_metres()
                {
                    differ.push(format!(
                        "volume ({} and {})",
                        Measure::Volume.describe((a.lower_cubic_metres(), a.upper_cubic_metres())),
                        Measure::Volume.describe((b.lower_cubic_metres(), b.upper_cubic_metres()))
                    ));
                } else if a.is_exact() && b.is_exact() {
                    agree.push("volume".to_owned());
                } else {
                    unknown.push("volume".to_owned());
                }
            }
            None => unknown.push("volume".to_owned()),
        }
        for quantity in &self.quantities {
            let property = PropertyRef {
                set: quantity.set.as_deref(),
                name: &quantity.name,
            };
            let read = |object: Option<&axioval_ir::Object>| -> Result<_, Unavailable> {
                let object = object.ok_or_else(|| invalid("an object is not in the project"))?;
                resolve(context, object, property)
            };
            match (read(objects.0), read(objects.1)) {
                (Ok(a), Ok(b)) => {
                    evidence.extend(a.evidence());
                    evidence.extend(b.evidence());
                    let key = |value: Option<&axioval_ir::PropertyValue>| {
                        value.map(|value| value_key(value, false, true))
                    };
                    if key(a.value()) == key(b.value()) {
                        agree.push(property.to_string());
                    } else {
                        differ.push(format!(
                            "{property} ({} and {})",
                            display(a.value()),
                            display(b.value())
                        ));
                    }
                }
                _ => unknown.push(property.to_string()),
            }
        }
        let mut suffix = if !differ.is_empty() {
            format!("; the copies differ in {}", differ.join(", "))
        } else if !agree.is_empty() {
            format!("; the copies agree in {}", agree.join(", "))
        } else {
            String::new()
        };
        if !unknown.is_empty() {
            suffix = format!(
                "{suffix}; whether they differ in {} is unknown",
                unknown.join(", ")
            );
        }
        (suffix, evidence)
    }
}
