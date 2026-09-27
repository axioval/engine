//! `space-connection`: which spaces a space may, must or must not open onto
//! directly, and whether it may, must or must not open to the outside.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId};

use crate::selection::{Selection, select_objects, selector_matches};
use crate::space_access::{AccessDeclaration, AccessIndex, AccessType, Exit, Link, unknown};
use crate::support::table::{Matched, RowSelection, RowTest, match_rows};
use crate::support::{Parameters, Unavailable, finding, invalid};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("from", ColumnKind::Selector),
    TableColumn::optional("to", ColumnKind::Selector),
    TableColumn::optional("access", ColumnKind::String),
    TableColumn::optional("access_type", ColumnKind::String),
    TableColumn::optional("exit", ColumnKind::String),
];

/// Checks the direct connections of each selected space against a table.
///
/// Every row of `connections` whose `from` selector picks the space applies
/// to it. `access` (`allowed` by default, `required` or `forbidden`) says
/// whether it must or must not have direct access to a space `to` picks,
/// through a shared door or opening of `access_type` (`any` by default,
/// `doors` or `openings`); `exit` says likewise whether it must or must not
/// open directly to the outside. Required access needs one such space;
/// forbidden access is found for every one.
///
/// Doors are what `door_selector` picks and openings what
/// `opening_selector` picks; `access_path` leads from each to the spaces it
/// connects (see `space_access`). The outside is known only through
/// `axioval:derived.adjacent-space`, so an `exit` requirement with a stated
/// relationship is an invalid declaration.
///
/// A requirement is met or broken only by links through elements surely of
/// the asked type to spaces `to` surely picks. An element of undecided type,
/// one whose spaces cannot be read, or a linked space `to` cannot decide
/// leaves a verdict it could change not evaluated.
pub struct SpaceConnection;

/// What a row asks of a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Requirement {
    Allowed,
    Required,
    Forbidden,
}

impl Requirement {
    fn parse(column: &str, value: Option<&str>) -> Result<Self, Unavailable> {
        match value.unwrap_or("allowed") {
            "allowed" => Ok(Self::Allowed),
            "required" => Ok(Self::Required),
            "forbidden" => Ok(Self::Forbidden),
            other => Err(invalid(format!(
                "`{column}` `{other}` is unsupported (allowed, required, forbidden)"
            ))),
        }
    }
}

struct Row<'a> {
    name: String,
    from: &'a Selector,
    to: Option<&'a Selector>,
    access: Requirement,
    access_type: AccessType,
    exit: Requirement,
}

fn rows<'a>(
    parameters: &Parameters<'a>,
    access: &AccessDeclaration<'_>,
) -> Result<Vec<Row<'a>>, Unavailable> {
    let table = parameters
        .table("connections")?
        .ok_or_else(|| invalid("parameter `connections` is required"))?;
    table
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let name = match row.text("label")? {
                Some(label) => format!("row {index} ({label})"),
                None => format!("row {index}"),
            };
            let from = row
                .selector("from")?
                .ok_or_else(|| invalid(format!("{name} has no `from`")))?;
            let to = row.selector("to")?;
            let requirement = Requirement::parse("access", row.text("access")?)?;
            let exit = Requirement::parse("exit", row.text("exit")?)?;
            let access_type = AccessType::parse(row.text("access_type")?)?;
            access.admits(access_type)?;
            if requirement != Requirement::Allowed && to.is_none() {
                return Err(invalid(format!(
                    "{name} requires or forbids access without `to`"
                )));
            }
            if exit != Requirement::Allowed && !access.sided {
                return Err(invalid(format!(
                    "{name} judges an exit to the outside, which only \
                     `axioval:derived.adjacent-space` records"
                )));
            }
            Ok(Row {
                name,
                from,
                to,
                access: requirement,
                access_type,
                exit,
            })
        })
        .collect()
}

impl RuleCapability for SpaceConnection {
    fn id(&self) -> &'static str {
        "axioval:capability.space-connection"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("connections", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::required("access_path", ParameterType::StringList),
            ParameterDescriptor::optional("door_selector", ParameterType::Selector),
            ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_selector", ParameterType::Selector),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let declared = AccessDeclaration::parse(&parameters).and_then(|access| {
            let access = access.ok_or_else(|| invalid("parameter `access_path` is required"))?;
            let rows = rows(&parameters, &access)?;
            Ok((access, rows))
        });
        let (access, rows) = match declared {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("space-connection: {message}"),
                );
            }
        };
        let index = access.index(context);
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        let mut judge = Judge {
            context,
            rule,
            index: &index,
            targets: BTreeMap::new(),
        };
        for space in spaces {
            let matched = match_rows(&rows, RowSelection::All, |row| {
                match selector_matches(context, row.from, space, &mut Vec::new()) {
                    Selection::Match => RowTest::Match(0),
                    Selection::NoMatch => RowTest::NoMatch,
                    Selection::NotEvaluated(..) => RowTest::Undecided,
                }
            });
            let Matched::Rows(applicable) = matched else {
                evaluation.push_object_not_evaluated(
                    space.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    "space-connection: whether a row's `from` picks this space is undecided",
                );
                continue;
            };
            for (index, row) in applicable {
                for outcome in [judge.access(space, index, row), judge.exit(space, row)] {
                    match outcome {
                        Ok(Some(found)) => evaluation.push_finding(found),
                        Ok(None) => {}
                        Err((reason, message)) => evaluation.push_object_not_evaluated(
                            space.id.clone(),
                            reason,
                            format!("space-connection {}: {message}", row.name),
                        ),
                    }
                }
            }
        }
        evaluation
    }
}

struct Judge<'r, 'c> {
    context: &'r RuleContext<'c>,
    rule: &'r CompiledRule,
    index: &'r AccessIndex,
    /// Whether a row's `to` picks a space, by row and space.
    targets: BTreeMap<(usize, ObjectId), Selection>,
}

impl Judge<'_, '_> {
    fn target(&mut self, index: usize, to: &Selector, space: &ObjectId) -> Selection {
        let context = self.context;
        self.targets
            .entry((index, space.clone()))
            .or_insert_with(|| match context.project.object(space) {
                Some(object) => selector_matches(context, to, object, &mut Vec::new()),
                None => Selection::NotEvaluated(
                    NotEvaluatedReason::InvalidEvidence,
                    format!("{space} is not in the project"),
                ),
            })
            .clone()
    }

    /// Judges the row's `access` requirement for `space`.
    fn access(
        &mut self,
        space: &Object,
        index: usize,
        row: &Row<'_>,
    ) -> Result<Option<Finding>, Unavailable> {
        let (Some(to), true) = (row.to, row.access != Requirement::Allowed) else {
            return Ok(None);
        };
        let partners = self.index.partners(&space.id, row.access_type);
        let mut sure: Vec<(ObjectId, ObjectId, Vec<Evidence>)> = Vec::new();
        let mut maybe: Vec<String> = partners.unknown.clone();
        for (other, link) in &partners.linked {
            match (self.target(index, to, other), link) {
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
        let kind = row.access_type.describe();
        let via = &self.index.relationship;
        match row.access {
            Requirement::Required if !sure.is_empty() => Ok(None),
            Requirement::Forbidden if sure.is_empty() && maybe.is_empty() => Ok(None),
            Requirement::Required if maybe.is_empty() => {
                let (elements, evidence) = self.index.cited(&space.id);
                Ok(Some(finding(
                    self.rule,
                    &space.id,
                    format!(
                        "has no direct access through a {kind} to a space {} requires \
                         (via {via})",
                        row.name
                    ),
                    evidence,
                    elements,
                )))
            }
            Requirement::Forbidden if !sure.is_empty() => {
                let links = sure
                    .iter()
                    .map(|(other, element, _)| format!("{other} through {element}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut evidence = Vec::new();
                let mut related = Vec::new();
                for (other, element, cited) in sure {
                    evidence.extend(cited);
                    related.push(other);
                    related.push(element);
                }
                Ok(Some(finding(
                    self.rule,
                    &space.id,
                    format!(
                        "has direct access to {links}, which {} forbids for a {kind}",
                        row.name
                    ),
                    evidence,
                    related,
                )))
            }
            _ => Err(unknown(maybe.join("; "))),
        }
    }

    /// Judges the row's `exit` requirement for `space`.
    fn exit(&self, space: &Object, row: &Row<'_>) -> Result<Option<Finding>, Unavailable> {
        let kind = row.access_type.describe();
        match (row.exit, self.index.exit(&space.id, row.access_type)) {
            (Requirement::Allowed, _)
            | (Requirement::Required, Exit::Sure { .. })
            | (Requirement::Forbidden, Exit::None) => Ok(None),
            (_, Exit::Unknown(why)) => Err(unknown(why)),
            (Requirement::Required, Exit::None) => {
                let (elements, evidence) = self.index.cited(&space.id);
                Ok(Some(finding(
                    self.rule,
                    &space.id,
                    format!(
                        "has no {kind} directly to the outside, which {} requires",
                        row.name
                    ),
                    evidence,
                    elements,
                )))
            }
            (Requirement::Forbidden, Exit::Sure { via, evidence }) => Ok(Some(finding(
                self.rule,
                &space.id,
                format!(
                    "opens directly to the outside through {via}, which {} forbids for a {kind}",
                    row.name
                ),
                evidence,
                vec![via],
            ))),
        }
    }
}
