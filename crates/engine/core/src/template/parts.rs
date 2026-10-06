//! The parts of each selected object judged as objects of their own
//! ([`Parts`]): a whole stair's flights.
//!
//! A form deciding [`Decision::Parts`](super::Decision) selects anchors (a
//! stair) and reaches their parts: the objects a selector parameter picks
//! that a path parameter reaches from the anchor. Each part is judged once,
//! by the first anchor reaching it, which its measured values may name
//! (`@anchor`): its own values are read first (one that cannot be read
//! leaves the part open once), then its checks, each outcome on the part.
//! The form's own values and checks then judge the anchor.

use serde::Serialize;

use super::{FormCheck, TemplateValue};

/// The parts of an anchor and how each is judged.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parts {
    /// The selector parameter picking parts.
    pub selector: &'static str,
    /// The string-list parameter naming the path from an anchor to its
    /// parts.
    pub path: &'static str,
    /// The values each part reads first, in order.
    pub values: Vec<TemplateValue>,
    /// The checks judging each part, each outcome on the part.
    pub checks: Vec<FormCheck>,
}
