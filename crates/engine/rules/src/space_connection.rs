//! `space-connection`: which spaces a space may, must or must not open onto
//! directly, and whether it may, must or must not open to the outside.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, ParameterDescriptor, RuleCapability,
    RuleContext, TableColumn,
};
use axioval_ir::contract::{ParameterValue, Selector};

use crate::space_access::{AccessDeclaration, AccessType};
use crate::support::{Parameters, Unavailable, invalid};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::ConnectionMeasures;

pub(crate) const COLUMNS: &[TableColumn] = &[
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
///
/// It runs as a template ([`axioval_engine::template`]): each applicable
/// row's requirements of a space, as the measured list
/// `space_connections` reads them (whether the space is surely linked,
/// surely not, or undecided), judged one by one.
pub struct SpaceConnection;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for SpaceConnection {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// What a row asks of a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Requirement {
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

/// One row of `connections`, as read.
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) from: Selector,
    pub(crate) to: Option<Selector>,
    pub(crate) access: Requirement,
    pub(crate) access_type: AccessType,
    pub(crate) exit: Requirement,
}

/// The rows of `connections`, each refused as the capability refused it
/// and, given `access`, checked against it row by row in the capability's
/// order (a provider reads rows its declaration check let through).
pub(crate) fn rows(
    parameters: &Parameters<'_>,
    access: Option<&AccessDeclaration<'_>>,
) -> Result<Vec<Row>, Unavailable> {
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
                .ok_or_else(|| invalid(format!("{name} has no `from`")))?
                .clone();
            let to = row.selector("to")?.cloned();
            let required = Requirement::parse("access", row.text("access")?)?;
            let exit = Requirement::parse("exit", row.text("exit")?)?;
            let access_type = AccessType::parse(row.text("access_type")?)?;
            if let Some(access) = access {
                access.admits(access_type)?;
                if required != Requirement::Allowed && to.is_none() {
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
            }
            Ok(Row {
                name,
                from,
                to,
                access: required,
                access_type,
                exit,
            })
        })
        .collect()
}

/// The declaration the capability refused, in its order and words: the
/// access path and selectors, then the rows. `stated` holds the rule's
/// parameters the list names, by the list's keys (the parameters' own
/// names).
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(stated.clone());
    let parameters = Parameters(&rule);
    let access = AccessDeclaration::parse(&parameters)?
        .ok_or_else(|| invalid("parameter `access_path` is required"))?;
    rows(&parameters, Some(&access)).map(|_| ())
}
