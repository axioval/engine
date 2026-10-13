//! The expression language's engine side: units, sound interval
//! arithmetic, evaluation with three-valued truth, and type checking.
//!
//! The contract is [`axioval_ir::contract::Expression`]; this module never
//! runs package code, and every expression terminates.
mod aggregate;
mod evaluate;
mod extreme;
mod interval;
mod sequence;
mod table;
mod text;
mod types;
mod unit;

pub use crate::catalogues::{catalogue_value, catalogue_value_at};
pub use crate::values::derived_value;
pub use evaluate::{
    DEFAULT_EVALUATION_BUDGET, Evaluation, EvaluationBudget, ExpressionContext, Leaf, Member,
    NotEvaluated, PairRead, Ranked, Read, Reason, RuleRead, Sequenced, Source, Value, ZoneNamed,
    evaluate, evaluate_untraced,
};
pub use interval::{Interval, IntervalFailure, IntervalResult};
pub use text::{MAX_TEXT_LENGTH, parse_text};
pub use types::{
    CatalogueTypes, DeclaredTypes, Type, TypeEnvironment, TypeError, TypeErrorKind,
    catalogue_column, check, check_as, measured_type,
};
pub use unit::{UNIT_SYMBOLS, Unit, UnitSymbol, parse_unit};
