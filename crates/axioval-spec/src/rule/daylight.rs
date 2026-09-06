use serde::{Deserialize, Serialize};

use super::assertion::Severity;

fn is_false(value: &bool) -> bool {
    !*value
}

/// One space exclusion row. Populated selectors are combined with AND.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaylightSpaceExclusionSpec {
    /// Canonical native filter identity. Providers expose exact acceptance facts
    /// under the same identity; the runtime still owns row conjunction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
}

/// A class-and-dimension lookup for effective light-opening area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaylightOpeningAreaRowSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    pub classification: String,
    pub width_m: f64,
    pub height_m: f64,
    pub area_m2: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DaylightOpeningPropertySpec {
    Identification(String),
    PropertySet {
        property_set: String,
        property: String,
    },
}

/// Vendor-neutral floor-to-light-opening ratio execution plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloorOpeningRatioPlanSpec {
    pub minimum_ratio: f64,
    pub maximum_ratio: f64,
    pub space_classification_scheme: String,
    pub opening_classification_scheme: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_filters: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_spaces: Vec<DaylightSpaceExclusionSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opening_area_rows: Vec<DaylightOpeningAreaRowSpec>,
    pub default_frame_width_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening_area_property: Option<DaylightOpeningPropertySpec>,
    /// When true, a provider-resolved light-opening property is preferred over
    /// table and frame-derived values.
    #[serde(default, skip_serializing_if = "is_false")]
    pub prefer_resolved_opening_area: bool,
    #[serde(default = "default_unavailable_severity")]
    pub unavailable_severity: Severity,
}

fn default_unavailable_severity() -> Severity {
    Severity::Warning
}

impl FloorOpeningRatioPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !self.minimum_ratio.is_finite()
            || !self.maximum_ratio.is_finite()
            || self.minimum_ratio < 0.0
            || self.maximum_ratio < self.minimum_ratio
        {
            return Err(
                "floor-opening ratio bounds must be finite, nonnegative, and ordered".into(),
            );
        }
        if !self.use_filters
            && (self.space_classification_scheme.trim().is_empty()
                || self.opening_classification_scheme.trim().is_empty())
        {
            return Err("floor-opening ratio classification schemes must be nonblank".into());
        }
        if self.use_filters
            && self
                .space_filter
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err("filter mode requires a nonblank space filter".into());
        }
        if !self.default_frame_width_m.is_finite() || self.default_frame_width_m < 0.0 {
            return Err("default frame width must be finite and nonnegative".into());
        }
        for (index, row) in self.opening_area_rows.iter().enumerate() {
            if (!self.use_filters && row.classification.trim().is_empty())
                || (self.use_filters
                    && row
                        .filter
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty()))
                || !row.width_m.is_finite()
                || !row.height_m.is_finite()
                || !row.area_m2.is_finite()
                || row.width_m < 0.0
                || row.height_m < 0.0
                || row.area_m2 < 0.0
            {
                return Err(format!("opening-area row {index} is invalid"));
            }
        }
        Ok(())
    }
}
