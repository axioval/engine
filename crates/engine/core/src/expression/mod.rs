//! The expression language's engine side: units, sound interval
//! arithmetic, evaluation with three-valued truth, and type checking.
//!
//! The contract is [`axioval_ir::contract::Expression`]; this module never
//! runs package code, and every expression terminates.
mod aggregate;
mod evaluate;
mod interval;
mod types;
mod unit;

pub use crate::values::derived_value;
pub use evaluate::{
    Evaluation, ExpressionContext, Leaf, Member, NotEvaluated, Read, Reason, RuleRead, Source,
    Value, evaluate,
};
pub use interval::{Interval, IntervalFailure, IntervalResult};
pub use types::{Type, TypeEnvironment, TypeError, TypeErrorKind, check, check_as, measured_type};
pub use unit::{Unit, parse_unit};
