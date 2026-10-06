//! Plan-span measurements as values, measured exactly as the capabilities
//! measure them: the distance from a component's centre line to the walls
//! beside it (`centre-line-distance`), a footprint's longest diagonal and
//! its exits' separations (`exit-separation`), its recesses
//! (`recess-width`), a light well's shared section, height and gaps
//! (`light-well`), and the end walls of the corridors an opening faces
//! (`corridor-end-openings`).

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CorridorEndRequest, EndWall, MeasuredMember, MeasuredProvider, Measurement, MemberValue,
    NotEvaluatedReason, PlanLength, PlanSpan, PlanSpanServiceHandle, PropertyResolutionError,
    RectangleSide, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId, QuantityDimension};

use crate::centre_line_distance::{Line, line};
use crate::exit_separation::{Separation, measure_pairs};
use crate::support::{Traversal, Unavailable};
use crate::wall_sides::{Nearest, Walls};

/// Measures plan-span values.
pub(crate) struct PlanMeasures;

const LENGTH: Option<QuantityDimension> = Some(QuantityDimension::Length);

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => *value,
        _ => 0.0,
    }
}

/// A value cited as exactly as the evidence it was measured from.
fn value(
    (lower, upper): (f64, f64),
    dimension: Option<QuantityDimension>,
    exact: bool,
    locator: String,
) -> Measurement {
    crate::measured_kinds::interval((lower, upper), dimension, exact, locator)
}

/// `minuend − subtrahend`, rounded outward: a point where it is exact.
fn difference(minuend: f64, subtrahend: f64) -> (f64, f64) {
    use axioval_engine::expression::Interval;
    Interval::point(minuend)
        .minus(Interval::point(subtrahend))
        .map_or((f64::NEG_INFINITY, f64::INFINITY), |difference| {
            (difference.lower, difference.upper)
        })
}

fn plan(length: &PlanLength, locator: String) -> Measurement {
    value(
        (length.lower_metres(), length.upper_metres()),
        LENGTH,
        length.evidence().exact,
        locator,
    )
}

fn spans<'a>(context: &RuleContext<'a>) -> Result<&'a PlanSpanServiceHandle, Unavailable> {
    context
        .services
        .get::<PlanSpanServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "plan-span service is not registered".to_owned(),
            )
        })
}

fn span_error(error: &axioval_engine::PlanSpanError) -> Unavailable {
    let reason = match error {
        axioval_engine::PlanSpanError::UnknownObject(_)
        | axioval_engine::PlanSpanError::Unavailable(_) => NotEvaluatedReason::IncompleteEvidence,
        _ => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, error.to_string())
}

/// The objects a `key` path reaches from `object`.
fn reached(
    call: &MeasuredCall,
    key: &str,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<ObjectId>, Unavailable> {
    let Some(MeasuredArgument::Path(steps)) = call.argument(key) else {
        return Err(crate::support::invalid(format!("`{key}` is required")));
    };
    let everything: Vec<&Object> = context.project.objects().collect();
    let (found, _) = Traversal::path(steps)?.related(context, object, &everything)?;
    Ok(found.into_iter().collect())
}

/// `objects`, of the `kinds` when the call states them.
fn of_kinds(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    mut objects: Vec<ObjectId>,
) -> Result<Vec<ObjectId>, Unavailable> {
    if call.argument("kinds").is_some() {
        let kinds: BTreeSet<ObjectId> =
            crate::measured_kinds::objects_of_kinds(context, call, "kinds", object)
                .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?;
        objects.retain(|found| kinds.contains(found));
    }
    Ok(objects)
}

/// The nearest wall beside one side as an interval: from every wall that
/// may be there to the nearest sure one, or just past `reach` when no wall
/// surely is; `None` when none may be.
fn side_interval(nearest: &Nearest, reach: f64) -> Option<(f64, f64)> {
    let lower = nearest.lower?;
    let upper = nearest
        .sure
        .as_ref()
        .map_or(reach.next_up(), |(_, _, upper)| *upper);
    Some((lower, upper.max(lower)))
}

impl PlanMeasures {
    fn centre_line(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let spans = spans(context)?;
        let walls = Walls::of(
            crate::measured_kinds::objects_of_kinds(context, call, "walls", object)
                .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?,
        );
        let reach = length(call, "reach");
        let measured = walls.measure(spans, object, reach, length(call, "inset"))?;
        let centre = match call.choice("centre_line") {
            Some("short") => Line::Short,
            Some("against-wall") => Line::AgainstWall,
            _ => Line::Long,
        };
        let (axis, _, cited) = line(centre, &walls, &measured)?;
        // Cited as `centre-line-distance` cites its sides.
        let exact = measured.evidence().exact
            && measured.rectangle().evidence().exact
            && cited.iter().all(|evidence| evidence.exact);
        let across = 1 - axis;
        let sides = [RectangleSide::ALL[across], RectangleSide::ALL[across + 2]]
            .map(|side| side_interval(&walls.nearest(&measured, side), reach));
        let locator = format!("{}:{object}", call.name());
        let combined = if call.choice("side") == Some("farther") {
            match sides {
                [Some(a), Some(b)] => Some((a.0.max(b.0), a.1.max(b.1))),
                _ => None,
            }
        } else {
            match sides {
                [Some(a), Some(b)] => Some((a.0.min(b.0), a.1.min(b.1))),
                [a, b] => a.or(b),
            }
        };
        Ok(match combined {
            Some(interval) => value(interval, LENGTH, exact, locator),
            None => Measurement::Absent {
                locator: format!("{locator}: no wall within the reach"),
            },
        })
    }

    fn well(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let members = reached(call, "members", object, context)?;
        if members.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "the path reaches no space".into(),
            ));
        }
        let locator = format!("{}:{object}", call.name());
        let name = call.name();
        if name == "well_section_area" || name == "well_section_width" {
            let section = spans(context)?
                .measure_section(&members)
                .map_err(|error| span_error(&error))?;
            if name == "well_section_area" {
                return Ok(value(
                    (section.area_lower(), section.area_upper()),
                    Some(QuantityDimension::Area),
                    section.evidence().exact,
                    locator,
                ));
            }
            return Ok(match section.width() {
                Some(width) => plan(width, locator),
                None => Measurement::Absent {
                    locator: format!("{locator}: the section has no width"),
                },
            });
        }
        let service = crate::level_spacing::extents(context)?;
        let mut stack = members
            .iter()
            .map(|member| crate::level_spacing::extent(service, member))
            .collect::<Result<Vec<_>, _>>()?;
        stack.sort_by(|a, b| {
            a.bottom()
                .lower_metres()
                .total_cmp(&b.bottom().lower_metres())
                .then_with(|| a.object().cmp(b.object()))
        });
        let exact = stack.iter().all(|member| member.evidence().exact);
        if name == "well_height" {
            let top = |pick: fn(&axioval_engine::VerticalExtent) -> f64| {
                stack.iter().map(pick).fold(f64::MIN, f64::max)
            };
            let bottom = |pick: fn(&axioval_engine::VerticalExtent) -> f64| {
                stack.iter().map(pick).fold(f64::MAX, f64::min)
            };
            // The differences round outward; keep the exact height inside.
            let lower = difference(
                top(|m| m.top().lower_metres()),
                bottom(|m| m.bottom().upper_metres()),
            )
            .0
            .max(0.0);
            let upper = difference(
                top(|m| m.top().upper_metres()),
                bottom(|m| m.bottom().lower_metres()),
            )
            .1;
            return Ok(value((lower, upper.max(lower)), LENGTH, exact, locator));
        }
        // The largest gap between consecutive members, bottom to top.
        let mut gap = (0.0_f64, 0.0_f64);
        for pair in stack.windows(2) {
            let (below, above) = (&pair[0], &pair[1]);
            // The differences round outward; keep the exact gap inside.
            let low = difference(above.bottom().lower_metres(), below.top().upper_metres()).0;
            let high = difference(above.bottom().upper_metres(), below.top().lower_metres()).1;
            gap = (gap.0.max(low.max(0.0)), gap.1.max(high.max(0.0)));
        }
        Ok(value(gap, LENGTH, exact, locator))
    }

    fn measure_object(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        match call.name() {
            "centre_line_distance" => Self::centre_line(call, object, context),
            "plan_diameter" => {
                let diameter = spans(context)?
                    .measure_diameter(object)
                    .map_err(|error| span_error(&error))?;
                Ok(plan(&diameter, format!("plan_diameter:{object}")))
            }
            _ => Self::well(call, object, context),
        }
    }

    /// The recesses of the footprint, each with the first row of the
    /// `requirements` handed in whose depth range holds it and the width
    /// that row requires: `null` where no row holds it (or none is handed
    /// in), undecided where its depth straddles a row's bound.
    fn recesses(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, Unavailable> {
        use crate::support::table::{Matched, RowSelection, match_rows};
        let rows = match call.argument("requirements") {
            Some(MeasuredArgument::Table(rows)) => Some(crate::recess_width::rows(rows)?),
            _ => None,
        };
        let found = spans(context)?
            .measure_recesses(object)
            .map_err(|error| crate::recess_width::unavailable(object, &error))?;
        let total = found.recesses().len();
        Ok(found
            .recesses()
            .iter()
            .enumerate()
            .map(|(index, recess)| {
                let at = |field: &str| format!("recesses:{object}#{}/{total}:{field}", index + 1);
                let (depth, width) = (recess.depth(), recess.width());
                let exact =
                    found.evidence().exact && depth.evidence().exact && width.evidence().exact;
                let absent = |field: &str| {
                    MemberValue::Measured(Measurement::Absent {
                        locator: format!("{}: no row holds the recess", at(field)),
                    })
                };
                let (row, required) = match rows.as_ref().map(|rows| {
                    match_rows(rows, RowSelection::First, |row| {
                        row.holds(depth.lower_metres(), depth.upper_metres())
                    })
                }) {
                    Some(Matched::Rows(matched)) => match matched.first() {
                        Some((number, row)) => {
                            #[allow(clippy::cast_precision_loss)]
                            let number = *number as f64;
                            (
                                MemberValue::Measured(value(
                                    (number, number),
                                    None,
                                    true,
                                    at("row"),
                                )),
                                MemberValue::Measured(value(
                                    row.required(depth.lower_metres(), depth.upper_metres()),
                                    LENGTH,
                                    depth.evidence().exact,
                                    at("required"),
                                )),
                            )
                        }
                        None => (absent("row"), absent("required")),
                    },
                    Some(Matched::Undecided | Matched::Ambiguous(_)) => {
                        let undecided = || MemberValue::Undecided {
                            why: "which row applies is undecided".into(),
                        };
                        (undecided(), undecided())
                    }
                    None => (absent("row"), absent("required")),
                };
                MeasuredMember {
                    certain: true,
                    exact,
                    fields: BTreeMap::from([
                        (
                            "place",
                            MemberValue::Text {
                                text: crate::recess_width::located(recess),
                            },
                        ),
                        ("width", MemberValue::Measured(plan(width, at("width")))),
                        ("depth", MemberValue::Measured(plan(depth, at("depth")))),
                        ("row", row),
                        ("required", required),
                    ]),
                }
            })
            .collect())
    }

    /// Every end wall of each corridor the `corridor` path reaches from the
    /// opening, with the opening's gap to it and the length it faces.
    fn end_walls(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, Unavailable> {
        let spans = spans(context)?;
        let mut members = Vec::new();
        for corridor in of_kinds(
            call,
            object,
            context,
            reached(call, "corridor", object, context)?,
        )? {
            let request = CorridorEndRequest::try_new(corridor.clone(), [object.clone()])
                .map_err(|error| span_error(&error))?;
            let ends = spans
                .measure_corridor_ends(&request)
                .map_err(|error| span_error(&error))?;
            for (index, end) in ends.ends().iter().enumerate() {
                let at =
                    |field: &str| format!("end_walls:{corridor}#{}:{object}:{field}", index + 1);
                let fields = match end.wall() {
                    EndWall::Undecided(why) => {
                        let undecided = || MemberValue::Undecided {
                            why: format!(
                                "the wall an end of {corridor} runs into is undecided: {why}"
                            ),
                        };
                        BTreeMap::from([("gap", undecided()), ("facing", undecided())])
                    }
                    EndWall::Decided { contacts, .. } => {
                        let contact = &contacts[0];
                        BTreeMap::from([
                            ("gap", MemberValue::Measured(plan(contact.gap(), at("gap")))),
                            (
                                "facing",
                                MemberValue::Measured(plan(contact.facing(), at("facing"))),
                            ),
                        ])
                    }
                };
                members.push(MeasuredMember {
                    certain: true,
                    exact: ends.evidence().exact,
                    fields,
                });
            }
        }
        Ok(members)
    }

    /// Every pair of the exits the `exits` path reaches, of the `kinds`
    /// when stated, with their separation.
    fn exit_pairs(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, Unavailable> {
        let exits = of_kinds(
            call,
            object,
            context,
            reached(call, "exits", object, context)?,
        )?;
        let separation = match call.choice("between") {
            Some("centres") => Separation::Span(PlanSpan::Centres),
            Some("farthest") => Separation::Span(PlanSpan::Farthest),
            _ => Separation::Closest,
        };
        let pairs = measure_pairs(context, spans(context)?, separation, &exits)?;
        Ok(pairs
            .into_iter()
            .map(|pair| {
                let at = format!(
                    "exit_pairs:{object}:{}:{}:separation",
                    pair.first, pair.second
                );
                let exact = pair
                    .measured
                    .as_ref()
                    .is_ok_and(|(_, _, cited)| cited.exact);
                let separation = match pair.measured {
                    Ok((lower, upper, _)) => {
                        MemberValue::Measured(value((lower, upper), LENGTH, exact, at))
                    }
                    Err(why) => MemberValue::Undecided {
                        why: format!(
                            "the separation of {} and {} cannot be measured: {why}",
                            pair.first, pair.second
                        ),
                    },
                };
                MeasuredMember {
                    certain: true,
                    exact,
                    fields: BTreeMap::from([("separation", separation)]),
                }
            })
            .collect())
    }
}

fn refused(
    call: &MeasuredCall,
    object: &ObjectId,
) -> impl Fn(Unavailable) -> PropertyResolutionError {
    let name = call.name();
    let object = object.clone();
    move |(reason, why)| {
        crate::measured_kinds::resolution_error((reason, format!("`{name}` of {object}: {why}")))
    }
}

impl MeasuredProvider for PlanMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            "centre_line_distance",
            "plan_diameter",
            "well_gap",
            "well_height",
            "well_section_area",
            "well_section_width",
        ]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &["end_walls", "exit_pairs", "recesses"]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Self::measure_object(call, object, context).map_err(refused(call, object))
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        match call.name() {
            "recesses" => Self::recesses(call, object, context),
            "end_walls" => Self::end_walls(call, object, context),
            _ => Self::exit_pairs(call, object, context),
        }
        .map_err(refused(call, object))
    }
}
