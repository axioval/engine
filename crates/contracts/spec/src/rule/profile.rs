use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProfileMatchPolicySpec {
    #[serde(default)]
    pub allow_cross_row_combinations: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllowedProfileRowSpec {
    pub values: Vec<Option<f64>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileDimensionKindSpec {
    Length,
    Ratio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDimensionSpec {
    pub id: String,
    pub kind: ProfileDimensionKindSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllowedProfileFamilySpec {
    pub family: String,
    pub dimensions: Vec<ProfileDimensionSpec>,
    pub rows: Vec<AllowedProfileRowSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllowedProfilePlanSpec {
    pub components: ElementScopeSpec,
    pub families: Vec<AllowedProfileFamilySpec>,
    pub tolerance_metres: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ratio_tolerance: f64,
    #[serde(default)]
    pub match_policy: ProfileMatchPolicySpec,
}

impl AllowedProfilePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !self.tolerance_metres.is_finite()
            || self.tolerance_metres < 0.0
            || !self.ratio_tolerance.is_finite()
            || self.ratio_tolerance < 0.0
        {
            return Err("profile tolerance must be finite and nonnegative".into());
        }
        if self.families.is_empty() {
            return Err("at least one allowed profile family is required".into());
        }
        for family in &self.families {
            if family.family.trim().is_empty()
                || family.dimensions.is_empty()
                || family
                    .dimensions
                    .iter()
                    .any(|dimension| dimension.id.trim().is_empty())
            {
                return Err("profile family and dimensions must be nonempty".into());
            }
            for row in &family.rows {
                if row.values.len() != family.dimensions.len() {
                    return Err(format!(
                        "profile family {} row width differs from its dimensions",
                        family.family
                    ));
                }
                if row.values.iter().flatten().any(|value| !value.is_finite()) {
                    return Err(format!(
                        "profile family {} contains a non-finite dimension",
                        family.family
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowedProfileFamilySchema {
    pub family: &'static str,
    pub dimensions: &'static [(&'static str, ProfileDimensionKindSpec)],
}

/// Canonical neutral schemas for the 18 IFC parametric profile families.
pub const ALLOWED_PROFILE_FAMILY_SCHEMAS: &[AllowedProfileFamilySchema] = &[
    AllowedProfileFamilySchema {
        family: "c_profile",
        dimensions: &[
            ("Depth", ProfileDimensionKindSpec::Length),
            ("Width", ProfileDimensionKindSpec::Length),
            ("WallThickness", ProfileDimensionKindSpec::Length),
            ("Girth", ProfileDimensionKindSpec::Length),
            ("InternalFilletRadius", ProfileDimensionKindSpec::Length),
            ("CentreOfGravityInX", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "i_profile",
        dimensions: &[
            ("OverallWidth", ProfileDimensionKindSpec::Length),
            ("OverallDepth", ProfileDimensionKindSpec::Length),
            ("WebThickness", ProfileDimensionKindSpec::Length),
            ("FlangeThickness", ProfileDimensionKindSpec::Length),
            ("FilletRadius", ProfileDimensionKindSpec::Length),
            ("FlangeEdgeRadius", ProfileDimensionKindSpec::Length),
            ("FlangeSlope", ProfileDimensionKindSpec::Ratio),
        ],
    },
    AllowedProfileFamilySchema {
        family: "asymmetric_i_profile",
        dimensions: &[
            ("OverallWidth", ProfileDimensionKindSpec::Length),
            ("OverallDepth", ProfileDimensionKindSpec::Length),
            ("WebThickness", ProfileDimensionKindSpec::Length),
            ("FlangeThickness", ProfileDimensionKindSpec::Length),
            ("FilletRadius", ProfileDimensionKindSpec::Length),
            ("TopFlangeWidth", ProfileDimensionKindSpec::Length),
            ("TopFlangeThickness", ProfileDimensionKindSpec::Length),
            ("TopFlangeFilletRadius", ProfileDimensionKindSpec::Length),
            ("CentreOfGravityInY", ProfileDimensionKindSpec::Length),
            ("BottomFlangeEdgeRadius", ProfileDimensionKindSpec::Length),
            ("BottomFlangeSlope", ProfileDimensionKindSpec::Ratio),
            ("TopFlangeEdgeRadius", ProfileDimensionKindSpec::Length),
            ("TopFlangeSlope", ProfileDimensionKindSpec::Ratio),
        ],
    },
    AllowedProfileFamilySchema {
        family: "z_profile",
        dimensions: &[
            ("Depth", ProfileDimensionKindSpec::Length),
            ("FlangeWidth", ProfileDimensionKindSpec::Length),
            ("WebThickness", ProfileDimensionKindSpec::Length),
            ("FlangeThickness", ProfileDimensionKindSpec::Length),
            ("FilletRadius", ProfileDimensionKindSpec::Length),
            ("EdgeRadius", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "t_profile",
        dimensions: &[
            ("Depth", ProfileDimensionKindSpec::Length),
            ("FlangeWidth", ProfileDimensionKindSpec::Length),
            ("WebThickness", ProfileDimensionKindSpec::Length),
            ("FlangeThickness", ProfileDimensionKindSpec::Length),
            ("FilletRadius", ProfileDimensionKindSpec::Length),
            ("FlangeEdgeRadius", ProfileDimensionKindSpec::Length),
            ("WebEdgeRadius", ProfileDimensionKindSpec::Length),
            ("WebSlope", ProfileDimensionKindSpec::Ratio),
            ("CentreOfGravityInY", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "l_profile",
        dimensions: &[
            ("Depth", ProfileDimensionKindSpec::Length),
            ("Width", ProfileDimensionKindSpec::Length),
            ("Thickness", ProfileDimensionKindSpec::Length),
            ("FilletRadius", ProfileDimensionKindSpec::Length),
            ("EdgeRadius", ProfileDimensionKindSpec::Length),
            ("LegSlope", ProfileDimensionKindSpec::Ratio),
            ("CentreOfGravityInX", ProfileDimensionKindSpec::Length),
            ("CentreOfGravityInY", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "u_profile",
        dimensions: &[
            ("Depth", ProfileDimensionKindSpec::Length),
            ("FlangeWidth", ProfileDimensionKindSpec::Length),
            ("WebThickness", ProfileDimensionKindSpec::Length),
            ("FlangeThickness", ProfileDimensionKindSpec::Length),
            ("FilletRadius", ProfileDimensionKindSpec::Length),
            ("EdgeRadius", ProfileDimensionKindSpec::Length),
            ("FlangeSlope", ProfileDimensionKindSpec::Ratio),
            ("CentreOfGravityInX", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "rectangle_profile",
        dimensions: &[
            ("XDim", ProfileDimensionKindSpec::Length),
            ("YDim", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "rectangle_hollow_profile",
        dimensions: &[
            ("XDim", ProfileDimensionKindSpec::Length),
            ("YDim", ProfileDimensionKindSpec::Length),
            ("WallThickness", ProfileDimensionKindSpec::Length),
            ("InnerFilletRadius", ProfileDimensionKindSpec::Length),
            ("OuterFilletRadius", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "trapezium_profile",
        dimensions: &[
            ("BottomXDim", ProfileDimensionKindSpec::Length),
            ("TopXDim", ProfileDimensionKindSpec::Length),
            ("YDim", ProfileDimensionKindSpec::Length),
            ("TopXOffset", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "circle_profile",
        dimensions: &[("Radius", ProfileDimensionKindSpec::Length)],
    },
    AllowedProfileFamilySchema {
        family: "circle_hollow_profile",
        dimensions: &[
            ("Radius", ProfileDimensionKindSpec::Length),
            ("WallThickness", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "ellipse_profile",
        dimensions: &[
            ("SemiAxis1", ProfileDimensionKindSpec::Length),
            ("SemiAxis2", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "rounded_rectangle_profile",
        dimensions: &[
            ("XDim", ProfileDimensionKindSpec::Length),
            ("YDim", ProfileDimensionKindSpec::Length),
            ("RoundingRadius", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "crane_rail_f_profile",
        dimensions: &[
            ("OverallHeight", ProfileDimensionKindSpec::Length),
            ("HeadWidth", ProfileDimensionKindSpec::Length),
            ("Radius", ProfileDimensionKindSpec::Length),
            ("HeadDepth2", ProfileDimensionKindSpec::Length),
            ("HeadDepth3", ProfileDimensionKindSpec::Length),
            ("WebThickness", ProfileDimensionKindSpec::Length),
            ("BaseDepth1", ProfileDimensionKindSpec::Length),
            ("BaseDepth2", ProfileDimensionKindSpec::Length),
            ("CentreOfGravityInY", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "crane_rail_a_profile",
        dimensions: &[
            ("OverallHeight", ProfileDimensionKindSpec::Length),
            ("BaseWidth2", ProfileDimensionKindSpec::Length),
            ("Radius", ProfileDimensionKindSpec::Length),
            ("HeadWidth", ProfileDimensionKindSpec::Length),
            ("HeadDepth2", ProfileDimensionKindSpec::Length),
            ("HeadDepth3", ProfileDimensionKindSpec::Length),
            ("WebThickness", ProfileDimensionKindSpec::Length),
            ("BaseWidth4", ProfileDimensionKindSpec::Length),
            ("BaseDepth1", ProfileDimensionKindSpec::Length),
            ("BaseDepth2", ProfileDimensionKindSpec::Length),
            ("BaseDepth3", ProfileDimensionKindSpec::Length),
            ("CentreOfGravityInY", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "non_uniform_t_profile",
        dimensions: &[
            ("Depth", ProfileDimensionKindSpec::Length),
            ("FlangeWidth", ProfileDimensionKindSpec::Length),
            ("WebThickness", ProfileDimensionKindSpec::Length),
            ("LeftFlangeThickness", ProfileDimensionKindSpec::Length),
            ("RightFlangeThickness", ProfileDimensionKindSpec::Length),
            ("FilletRadius", ProfileDimensionKindSpec::Length),
            ("FlangeEdgeRadius", ProfileDimensionKindSpec::Length),
            ("WebEdgeRadius", ProfileDimensionKindSpec::Length),
            ("WebSlope", ProfileDimensionKindSpec::Ratio),
            ("WebOffset", ProfileDimensionKindSpec::Length),
            ("CentreOfGravityInY", ProfileDimensionKindSpec::Length),
        ],
    },
    AllowedProfileFamilySchema {
        family: "non_uniform_l_profile",
        dimensions: &[
            ("Depth", ProfileDimensionKindSpec::Length),
            ("Width", ProfileDimensionKindSpec::Length),
            ("Thickness", ProfileDimensionKindSpec::Length),
            ("FlangeThickness", ProfileDimensionKindSpec::Length),
            ("FilletRadius", ProfileDimensionKindSpec::Length),
            ("EdgeRadius", ProfileDimensionKindSpec::Length),
            ("LegSlope", ProfileDimensionKindSpec::Ratio),
            ("CentreOfGravityInX", ProfileDimensionKindSpec::Length),
            ("CentreOfGravityInY", ProfileDimensionKindSpec::Length),
        ],
    },
];
