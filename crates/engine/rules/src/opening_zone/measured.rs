//! An opening's placements in its hosts as measured members, placed and
//! measured as `opening-zone` places and measures them: whether it lies
//! within the host, and its clear distances from the host's ends and edges
//! (or flanges).

use std::borrow::Cow;
use std::collections::BTreeMap;

use axioval_engine::{
    CompiledRule, MeasuredMember, MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId, QuantityDimension, RuleId};

use super::{Clearance, Config, Host, Judge, Placed, Span, dimensions};
use crate::counts::Population;
use crate::measured_kinds::{objects_of_kinds, resolution_error};
use crate::support::{Unavailable, invalid};

/// The member list measured here.
const OPENING_PLACEMENTS: &str = "opening_placements";

const LENGTH: Option<QuantityDimension> = Some(QuantityDimension::Length);

/// Measures an opening's placements in its hosts.
pub(crate) struct PlacementMeasures;

/// The `opening-zone` declaration `call` states, its hosts the objects of
/// the `hosts` kinds when it names any.
fn declaration(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
    opening: &ObjectId,
) -> Result<CompiledRule, Unavailable> {
    let text = |value: &str| ParameterValue::String {
        value: value.to_owned(),
    };
    let Some(MeasuredArgument::Path(steps)) = call.argument("host_path") else {
        return Err(invalid("`host_path` is required"));
    };
    let mut parameters = BTreeMap::from([
        (
            "host_path".to_owned(),
            ParameterValue::StringList {
                value: steps.clone(),
            },
        ),
        (
            "length_axis".to_owned(),
            text(call.choice("length_axis").unwrap_or("extrusion")),
        ),
        (
            "height_axis".to_owned(),
            text(call.choice("height_axis").unwrap_or("profile-y")),
        ),
        (
            "zone".to_owned(),
            text(call.choice("zone").unwrap_or("section")),
        ),
    ]);
    if call.argument("hosts").is_some() {
        let hosts = objects_of_kinds(context, call, "hosts", opening)
            .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
        parameters.insert(
            "host_selector".to_owned(),
            ParameterValue::Selector {
                value: Box::new(Selector::Objects { objects: hosts }),
            },
        );
    }
    if let Some(MeasuredArgument::Number(minimum)) = call.argument("minimum") {
        parameters.insert(
            "minimum_opening_area".to_owned(),
            ParameterValue::Quantity {
                value: *minimum,
                unit: "m2".into(),
            },
        );
    }
    Ok(CompiledRule {
        id: RuleId::new("axioval-measured-opening-placements").expect("a valid rule id"),
        capability: "axioval:capability.opening-zone".into(),
        severity: Severity::Info,
        selector: Selector::All,
        parameters,
    })
}

/// A distance exact or known only from below: a lower bound stands for
/// every distance from it up to `span`, the host's whole extent, which no
/// clear distance within the host exceeds.
fn distance(clear: f64, exact: bool, span: Span, locator: &str) -> MemberValue {
    MemberValue::Measured(Measurement::Value {
        lower: clear,
        upper: if exact {
            clear
        } else {
            clear.max(span.1 - span.0)
        },
        dimension: LENGTH,
        locator: locator.to_owned(),
    })
}

fn absent(locator: &str) -> MemberValue {
    MemberValue::Measured(Measurement::Absent {
        locator: locator.to_owned(),
    })
}

/// Every field of a placement that cannot be measured, undecided.
fn undecided(why: &str) -> BTreeMap<&'static str, MemberValue> {
    FIELDS
        .iter()
        .map(|field| {
            (
                *field,
                MemberValue::Undecided {
                    why: why.to_owned(),
                },
            )
        })
        .collect()
}

const FIELDS: [&str; 5] = [
    "inside",
    "end_distance",
    "edge_distance",
    "bottom_distance",
    "top_distance",
];

impl Judge<'_, '_> {
    /// The fields of one placement, measured as [`Judge::judge_in`]
    /// measures them.
    fn fields(&self, host: &Host, placed: &Placed) -> BTreeMap<&'static str, MemberValue> {
        if placed.may_be_small {
            return undecided("whether its area is below the minimum is undecided");
        }
        let locator = format!("{OPENING_PLACEMENTS}:{}", placed.host);
        let axes = self.config.axes;
        let rect = placed.section_rect(host, axes);
        let (outside_length, outside_height) = self.beyond(host, placed);
        let inside = if outside_length || outside_height {
            Some(false)
        } else {
            Self::crossing(host, placed, rect).map(|crosses| !crosses)
        };
        let (_, length_span) = host.axis(axes.length);
        let (_, height_span) = host.axis(axes.height);
        let mut fields = BTreeMap::new();
        fields.insert(
            "inside",
            inside.map_or_else(
                || MemberValue::Undecided {
                    why: "it may cross the edge of its host's outline".to_owned(),
                },
                |value| MemberValue::Truth {
                    value,
                    locator: locator.clone(),
                },
            ),
        );
        fields.insert(
            "end_distance",
            self.end_clearance(host, placed, rect).map_or_else(
                || absent(&locator),
                |(clear, exact)| distance(clear, exact, length_span, &locator),
            ),
        );
        fields.insert(
            "edge_distance",
            match self.edge_clearance(host, placed, rect) {
                Ok(Some(Clearance { clear, exact, .. })) => {
                    distance(clear, exact, height_span, &locator)
                }
                Ok(None) => absent(&locator),
                Err((_, why)) => MemberValue::Undecided { why },
            },
        );
        let far = self.far_clearance(host, placed, rect);
        for (field, low) in [("bottom_distance", true), ("top_distance", false)] {
            let value = match &far {
                Ok(Some(((bottom, top), exact))) => distance(
                    if low { *bottom } else { *top },
                    *exact,
                    height_span,
                    &locator,
                ),
                Ok(None) => absent(&locator),
                Err((_, why)) => MemberValue::Undecided { why: why.clone() },
            };
            fields.insert(field, value);
        }
        fields
    }
}

impl PlacementMeasures {
    fn placements(
        call: &MeasuredCall,
        opening: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, Unavailable> {
        let rule = declaration(call, context, &opening.id)?;
        let config = Config::parse(&rule)?;
        let hosts = Population::of(context, config.host_selector);
        let mut judge = Judge {
            dimensions: &dimensions::Selections::of(context, &config.dimensions),
            context,
            #[cfg(feature = "parity-reference")]
            rule: &rule,
            config: &config,
            hosts: &hosts,
            bodies: Cow::Owned(BTreeMap::new()),
            placed: Cow::Owned(BTreeMap::new()),
            supports: None,
        };
        let placements = judge.place(opening)?;
        // No placement read: the opening is not measured, for the first
        // placement's reason; a host among others that cannot be read is an
        // undecided member.
        if let Some(Err(unread)) = placements.first()
            && placements.iter().all(Result::is_err)
        {
            return Err(unread.clone());
        }
        Ok(placements
            .iter()
            .map(|placement| {
                let host = placement.as_ref().ok().and_then(|placed| {
                    judge
                        .bodies
                        .get(&placed.host)
                        .and_then(|host| host.as_ref().ok())
                });
                match (placement, host) {
                    (Ok(placed), Some(host)) => MeasuredMember {
                        certain: true,
                        exact: placed.evidence.iter().all(|evidence| evidence.exact),
                        fields: judge.fields(host, placed),
                        evidence: Vec::new(),
                    },
                    (Err((_, why)), _) => MeasuredMember {
                        certain: true,
                        exact: false,
                        fields: undecided(why),
                        evidence: Vec::new(),
                    },
                    (Ok(_), None) => MeasuredMember {
                        certain: true,
                        exact: false,
                        fields: undecided("its host was not read"),
                        evidence: Vec::new(),
                    },
                }
            })
            .collect())
    }
}

impl MeasuredProvider for PlacementMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[OPENING_PLACEMENTS]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let opening = context.project.object(object).ok_or_else(|| {
            PropertyResolutionError::Unavailable(format!("{object} is not in the project"))
        })?;
        Self::placements(call, opening, context).map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of {object}: {why}", call.name())))
        })
    }
}
