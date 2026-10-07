//! A space's connections as measured members, read as `space-connection`
//! reads them: one item per requirement of each row of `connections` whose
//! `from` picks the space (its `access`, then its `exit`), stating whether
//! the space is surely linked as the row asks (to a space `to` picks
//! through an element of the row's type, or to the outside), surely not,
//! or undecided, with the links and elements a finding names.
//!
//! The access index is built once per run for the path and selections, and
//! the rows are read once per run for the table; whether a row's `to`
//! picks a space is decided once per run.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    ArgumentsKey, MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement, MemberValue,
    NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, Object, ObjectId};

use super::{Requirement, Row, rows};
use crate::measured_kinds::{resolution_error, selection};
use crate::selection::{Selection, selector_matches};
use crate::space_access::{AccessDeclaration, AccessIndex, Exit, Link, Pick};
use crate::support::table::{Matched, RowSelection, RowTest, match_rows};
use crate::support::{Parameters, Unavailable, invalid};

/// The member list measured here.
const SPACE_CONNECTIONS: &str = "space_connections";

/// Measures each space's connections against the rows that apply to it.
pub(crate) struct ConnectionMeasures;

/// The keys the access index depends on.
const ACCESS: &[&str] = &[
    "access_path",
    "door_selector",
    "opening_selector",
    "space_selector",
];

/// The access index the call's path and selections declare, built once per
/// run for them.
fn index(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<Arc<AccessIndex>, Unavailable> {
    let key = ArgumentsKey::of_keys(call, ACCESS);
    MeasuredMemo::of(context.services, key, || {
        let picked = |key: &str| -> Result<Option<MeasuredSelection>, Unavailable> {
            selection(context, call, key, None).map_err(crate::selection::property_error)
        };
        let (doors, openings, spaces) = (
            picked("door_selector")?,
            picked("opening_selector")?,
            picked("space_selector")?,
        );
        let Some(MeasuredArgument::Path(steps)) = call.argument("access_path") else {
            return Err(invalid("parameter `access_path` is required"));
        };
        let access = AccessDeclaration::of(
            steps,
            doors.as_ref().map(Pick::Selected),
            openings.as_ref().map(Pick::Selected),
            spaces.as_ref().map(Pick::Selected),
        )?;
        Ok(Arc::new(access.index(context)))
    })
}

/// The key of the rows the call's table states in the run's memo.
#[derive(Hash, PartialEq, Eq)]
struct RowsKey(ArgumentsKey);

/// The rows of the call's table, read once per run: the declaration check
/// refused any the capability refused.
fn table(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<Arc<Vec<Row>>, Unavailable> {
    let key = RowsKey(ArgumentsKey::of_keys(call, &["connections"]));
    MeasuredMemo::of(context.services, key, || {
        let Some(MeasuredArgument::Table(stated)) = call.argument("connections") else {
            return Err(invalid("parameter `connections` is required"));
        };
        let rule = crate::light_area::synthesised(BTreeMap::from([(
            "connections".to_owned(),
            ParameterValue::Table {
                value: stated.clone(),
            },
        )]));
        rows(&Parameters(&rule), None).map(Arc::new)
    })
}

/// The key of whether row `1` of the table `0` picks the space `2` as its
/// `to`.
#[derive(Hash, PartialEq, Eq)]
struct TargetKey(ArgumentsKey, usize, ObjectId);

/// Whether the row's `to` picks `space`, decided once per run.
fn target(
    context: &RuleContext<'_>,
    rows: &ArgumentsKey,
    (index, row): (usize, &Row),
    space: &ObjectId,
) -> Selection {
    let Some(to) = &row.to else {
        return Selection::NoMatch;
    };
    MeasuredMemo::of(
        context.services,
        TargetKey(rows.clone(), index, space.clone()),
        || match context.project.object(space) {
            Some(object) => selector_matches(context, to, object, &mut Vec::new()),
            None => Selection::NotEvaluated(
                NotEvaluatedReason::InvalidEvidence,
                format!("{space} is not in the project"),
            ),
        },
    )
}

/// One item of the list: the row's requirement and what the space's links
/// say of it.
struct Item {
    row: String,
    access: bool,
    required: bool,
    kind: &'static str,
    /// `Ok(true)` surely linked, `Ok(false)` surely not, `Err` undecided.
    linked: Result<bool, String>,
    links: String,
    related: Vec<ObjectId>,
    /// Whether every evidence the item was read from is exact.
    exact: bool,
}

impl Item {
    fn member(self, space: &ObjectId, via: &str) -> MeasuredMember {
        let truth = |value: bool| MemberValue::Truth {
            value,
            locator: format!("{SPACE_CONNECTIONS}:{space}"),
        };
        let text = |text: String| MemberValue::Text { text };
        let mut related = self.related;
        related.sort();
        related.dedup();
        MeasuredMember {
            certain: true,
            exact: self.exact,
            fields: [
                ("row", text(self.row)),
                ("access", truth(self.access)),
                ("required", truth(self.required)),
                ("kind", text(self.kind.to_owned())),
                ("via", text(via.to_owned())),
                ("links", text(self.links)),
                (
                    "linked",
                    match self.linked {
                        Ok(value) => truth(value),
                        Err(why) => MemberValue::Undecided { why },
                    },
                ),
                ("related", MemberValue::Objects { objects: related }),
            ]
            .into_iter()
            .collect(),
        }
    }
}

impl ConnectionMeasures {
    fn connections(
        call: &MeasuredCall,
        space: &Object,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
        let index = index(call, context)?;
        let table = table(call, context)?;
        let rows_key = ArgumentsKey::of_keys(call, &["connections"]);
        let matched = match_rows(&table, RowSelection::All, |row| {
            match selector_matches(context, &row.from, space, &mut Vec::new()) {
                Selection::Match => RowTest::Match(0),
                Selection::NoMatch => RowTest::NoMatch,
                Selection::NotEvaluated(..) => RowTest::Undecided,
            }
        });
        let Matched::Rows(applicable) = matched else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "whether a row's `from` picks this space is undecided".to_owned(),
            ));
        };
        let mut items = Vec::new();
        let mut evidence = Vec::new();
        for (position, row) in applicable {
            if row.access != Requirement::Allowed {
                items.push(access(
                    context,
                    (&index, &rows_key),
                    (position, row),
                    &space.id,
                    &mut evidence,
                ));
            }
            if row.exit != Requirement::Allowed {
                items.push(exit(&index, row, &space.id, &mut evidence));
            }
        }
        let via = index.relationship.as_str();
        Ok((
            items
                .into_iter()
                .map(|item| item.member(&space.id, via))
                .collect(),
            evidence,
        ))
    }
}

/// The row's `access` requirement of `space`: linked where some space
/// `to` surely picks is surely reached through an element of the row's
/// type, not linked where none may be, undecided otherwise.
fn access(
    context: &RuleContext<'_>,
    (index, rows): (&AccessIndex, &ArgumentsKey),
    (position, row): (usize, &Row),
    space: &ObjectId,
    evidence: &mut Vec<Evidence>,
) -> Item {
    let partners = index.partners(space, row.access_type);
    let mut sure: Vec<(ObjectId, ObjectId, Vec<Evidence>)> = Vec::new();
    let mut maybe: Vec<String> = partners.unknown.clone();
    for (other, link) in &partners.linked {
        match (target(context, rows, (position, row), other), link) {
            (Selection::NoMatch, _) => {}
            (Selection::Match, Link::Sure { via, evidence }) => {
                sure.push((other.clone(), via.clone(), evidence.clone()));
            }
            (Selection::NotEvaluated(_, why), Link::Sure { via, .. }) => maybe.push(format!(
                "{other}, reached through {via}, may be a space `to` picks: {why}"
            )),
            (_, Link::Maybe(why)) => maybe.push(format!("{other} may be linked: {why}")),
        }
    }
    let links = sure
        .iter()
        .map(|(other, element, _)| format!("{other} through {element}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut exact = true;
    let (linked, related) = if sure.is_empty() {
        let (elements, cited) = index.cited(space);
        exact = exactly(&cited);
        evidence.extend(cited);
        let linked = if maybe.is_empty() {
            Ok(false)
        } else {
            Err(maybe.join("; "))
        };
        (linked, elements)
    } else {
        let mut related = Vec::new();
        for (other, element, cited) in sure {
            exact &= exactly(&cited);
            evidence.extend(cited);
            related.push(other);
            related.push(element);
        }
        (Ok(true), related)
    };
    Item {
        row: row.name.clone(),
        access: true,
        required: row.access == Requirement::Required,
        kind: row.access_type.describe(),
        linked,
        links,
        related,
        exact,
    }
}

/// Whether every one of `evidence` is exact.
fn exactly(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
}

/// The row's `exit` requirement of `space`: linked where it surely opens to
/// the outside through an element of the row's type.
fn exit(index: &AccessIndex, row: &Row, space: &ObjectId, evidence: &mut Vec<Evidence>) -> Item {
    let (linked, links, related, exact) = match index.exit(space, row.access_type) {
        Exit::Sure {
            via,
            evidence: cited,
        } => {
            let exact = exactly(&cited);
            evidence.extend(cited);
            (Ok(true), via.to_string(), vec![via], exact)
        }
        Exit::None => {
            let (elements, cited) = index.cited(space);
            let exact = exactly(&cited);
            evidence.extend(cited);
            (Ok(false), String::new(), elements, exact)
        }
        Exit::Unknown(why) => (Err(why), String::new(), Vec::new(), true),
    };
    Item {
        row: row.name.clone(),
        access: false,
        required: row.exit == Requirement::Required,
        kind: row.access_type.describe(),
        linked,
        links,
        related,
        exact,
    }
}

impl MeasuredProvider for ConnectionMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[SPACE_CONNECTIONS]
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
        self.members_cited(call, object, context)
            .map(|(members, _)| members)
    }

    /// Each item's finding cites the relationship evidence its space's
    /// links rest on.
    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let space = context.project.object(object).ok_or_else(|| {
            PropertyResolutionError::Unavailable(format!("{object} is not in the project"))
        })?;
        Self::connections(call, space, context).map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of {object}: {why}", call.name())))
        })
    }
}
