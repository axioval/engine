use super::{
    CheckRuleSpec, CheckSemantics, ElementField, ElementScopeOp, PredicateValue,
    PropertyPredicateOp,
};

impl CheckRuleSpec {
    /// Validate invariants that every format adapter must satisfy before runtime
    /// compilation. Source-specific shape checks stay in the adapter; this
    /// method only enforces neutral semantic validity.
    pub fn validate(&self) -> Result<(), String> {
        if let CheckSemantics::Composite { checks } = &self.semantics {
            if checks.is_empty() {
                return Err("composite semantic check has no children".into());
            }
            let mut ids = std::collections::BTreeSet::new();
            for (index, child) in checks.iter().enumerate() {
                if child.id.trim().is_empty() {
                    return Err(format!("composite child {index} has an empty id"));
                }
                if !ids.insert(child.id.as_str()) {
                    return Err(format!(
                        "composite semantic check has duplicate id `{}`",
                        child.id
                    ));
                }
                CheckRuleSpec {
                    definition_id: self.definition_id.clone(),
                    id: child.id.clone(),
                    name: child.name.clone(),
                    semantics: child.semantics.clone(),
                }
                .validate()
                .map_err(|message| format!("composite child `{}`: {message}", child.id))?;
            }
            return Ok(());
        }
        if let CheckSemantics::RelationCardinality { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::SelectionCardinality { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::SpaceComponentCount { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::FloorOpeningRatio { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::RelativeCount { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::RequiredComponents { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::SpacesInDerivedGroups { plan } = &self.semantics {
            plan.validate()?;
        }
        if let CheckSemantics::SpaceGroupContainment { plan } = &self.semantics {
            plan.validate()?;
        }
        if let CheckSemantics::FireCompartmentArea { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::StoreyNameSequence { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::PairwiseGeometry { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ComponentDistance { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::FloorDistance { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::WallDistance { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::PathGeometry { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::RouteComponentCompliance { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::AccessibleSpace { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::BeamIntersectionCompliance { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::AllowedProfiles { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ArchitectureStructureCoverage { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::BuildingEnvelope { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ElementHolePlacement { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ComponentContainment { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ComponentVisibility { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ExitAccessDoorwayArrangement { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::FireWallComponents { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::LayerAgreement { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ExternalWallValidation { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::SpaceValidation { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::SlabContact { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::WallValidation { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::FrontClearance { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ComponentClearance { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::EffectiveCoverage { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::Parking { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::LocalCirculation { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::Ramp { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::Stair { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ModelComparison { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ModelArchitecture { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::BuildingStorey { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ConditionalPresence { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ElementValidation { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ElementDimension { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::TypeGroupSizeOutlier { plan } = &self.semantics {
            plan.validate()?;
        }
        if let CheckSemantics::WindowFloorRatio { plan } = &self.semantics {
            plan.validate()?;
        }

        if let CheckSemantics::ClashMatrix { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::EscapeRoute { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::FreeFloorSpace { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::StructureArchitectureConformity { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::SpaceDistances { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::OpeningSill { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::DoorAccessibility { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::FireCompartmentMembership { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::HorizontalGuard { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ManualIssues { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::PropertyComparison { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::ShelfCapacity { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::SpaceConnection { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::CirculationGeometry { plan } = &self.semantics {
            return plan.validate();
        }
        if let CheckSemantics::AgreedTypeValues {
            selector: _,
            rows,
            case_sensitive: _,
            severity: _,
        } = &self.semantics
        {
            if rows.is_empty() {
                return Err("agreed type-value check has no configured rows".into());
            }
            for (index, row) in rows.iter().enumerate() {
                if row.applies_to.trim().is_empty() || row.allowed_values.is_empty() {
                    return Err(format!("agreed type-value row {index} is empty"));
                }
                for value in &row.allowed_values {
                    if value.trim().is_empty() {
                        return Err(format!(
                            "allowed_values row {index} is empty; refusing a no-op agreed type value"
                        ));
                    }
                }
            }
            return Ok(());
        }
        if let CheckSemantics::ConsistentProperty {
            pairs,
            scope: _,
            element_scope: _,
            severity: _,
        } = &self.semantics
        {
            if pairs.is_empty() {
                return Err("consistent-property check has no configured pairs".into());
            }
            for (index, pair) in pairs.iter().enumerate() {
                // Comparing a property with itself is a tautology: every
                // component that agrees on X trivially agrees on X, so the
                // rule can never report anything. Native authoring allows it;
                // executing it as a silent no-op does not.
                if pair.compared == pair.identical {
                    return Err(format!(
                        "consistent-property pair {index} compares a property with itself"
                    ));
                }
            }
            return Ok(());
        }
        if let CheckSemantics::AgreedSpaceTypes {
            rows,
            case_sensitive: _,
            allow_whitespace: _,
            group_mode: _,
            severity: _,
        } = &self.semantics
        {
            for (index, row) in rows.iter().enumerate() {
                // Native getMatchingRow skips fully blank rows. Source codecs
                // normalize those away; explicit blank neutral rows are invalid.
                if row.is_blank() {
                    return Err(format!(
                        "agreed space-type row {index} has no populated column"
                    ));
                }
            }
            return Ok(());
        }
        if matches!(self.semantics, CheckSemantics::SpacePropertyPresence { .. }) {
            return Ok(());
        }
        if let CheckSemantics::SpaceTypeSizeCount {
            classification_name,
            requirements,
            ..
        } = &self.semantics
        {
            if requirements.is_empty() {
                return Err("space type-size count has no configured requirements".into());
            }
            for (index, requirement) in requirements.iter().enumerate() {
                if requirement.classification_pattern.trim().is_empty()
                    && requirement.space_type_pattern.trim().is_empty()
                    && requirement.space_name_pattern.trim().is_empty()
                    && requirement.space_number_pattern.trim().is_empty()
                {
                    return Err(format!(
                        "space type-size requirement {index} has no configured selector"
                    ));
                }
                if !requirement.classification_pattern.trim().is_empty()
                    && classification_name
                        .as_deref()
                        .is_none_or(|name| name.trim().is_empty())
                {
                    return Err(format!(
                        "space type-size requirement {index} uses classification without a scheme"
                    ));
                }
                if !requirement.target_area_m2.is_finite()
                    || !requirement.tolerance_fraction.is_finite()
                    || requirement.target_area_m2 < 0.0
                    || requirement.tolerance_fraction < 0.0
                {
                    return Err(format!(
                        "space type-size requirement {index} has invalid area or tolerance"
                    ));
                }
            }
            return Ok(());
        }
        if let CheckSemantics::SpaceArea {
            min_area_m2,
            max_area_m2,
        } = &self.semantics
        {
            if min_area_m2.is_none() && max_area_m2.is_none() {
                return Err("space-area check has no configured bound".into());
            }
            for (name, value) in [
                ("minimum", min_area_m2.as_ref()),
                ("maximum", max_area_m2.as_ref()),
            ] {
                if value.is_some_and(|value| !value.is_finite() || *value < 0.0) {
                    return Err(format!(
                        "space-area {name} bound must be finite and non-negative"
                    ));
                }
            }
            if min_area_m2
                .zip(*max_area_m2)
                .is_some_and(|(minimum, maximum)| minimum > maximum)
            {
                return Err("space-area minimum bound exceeds maximum bound".into());
            }
            return Ok(());
        }
        if let CheckSemantics::StoreySpaceCountAggregate {
            classification_name,
            requirements,
            ..
        } = &self.semantics
        {
            if requirements.is_empty() {
                return Err("storey space-count aggregate has no configured requirements".into());
            }
            for (index, requirement) in requirements.iter().enumerate() {
                if requirement.storey_name_pattern.trim().is_empty() {
                    return Err(format!(
                        "storey_name_pattern row {index} is empty; refusing a widened storey space-count check"
                    ));
                }
                if requirement.required_count == 0 {
                    return Err(format!(
                        "storey space-count requirement {index} has a zero required count"
                    ));
                }
                if !matches!(requirement.classification_pattern.trim(), "" | "*")
                    && classification_name.is_none()
                {
                    return Err(format!(
                        "storey space-count requirement {index} selects a classification but no classification is configured"
                    ));
                }
                if [
                    &requirement.classification_pattern,
                    &requirement.space_type_pattern,
                    &requirement.space_name_pattern,
                    &requirement.space_number_pattern,
                ]
                .iter()
                .all(|value| value.trim().is_empty())
                {
                    return Err(format!(
                        "storey space-count requirement {index} has no space selector"
                    ));
                }
            }
            return Ok(());
        }
        if let CheckSemantics::StoreySpaceAreaAggregate { limits } = &self.semantics {
            if limits.is_empty() {
                return Err("storey space-area aggregate has no configured limits".into());
            }
            for (index, limit) in limits.iter().enumerate() {
                if limit.storey_name_pattern.trim().is_empty() {
                    return Err(format!(
                        "storey_name_pattern row {index} is empty; refusing a widened storey space-area check"
                    ));
                }
                if !limit.min_area_m2.is_finite()
                    || !limit.max_area_m2.is_finite()
                    || limit.min_area_m2 < 0.0
                    || limit.max_area_m2 < limit.min_area_m2
                {
                    return Err(format!(
                        "storey space-area limit {index} has invalid square-metre bounds"
                    ));
                }
            }
            return Ok(());
        }
        let CheckSemantics::PropertyPredicates {
            selector,
            fallback_type,
            requirements,
            non_evaluating_requirements,
            ..
        } = &self.semantics
        else {
            // Phase 1 semantics retain their established compiler validation so
            // this family slice cannot change the original four behaviours.
            return Ok(());
        };
        if fallback_type.trim().is_empty() {
            return Err("property-predicate fallback type is empty".into());
        }
        for (index, clause) in selector.clauses.iter().enumerate() {
            if clause.relation.is_some()
                && (clause.field.is_some()
                    || !matches!(
                        clause.op,
                        ElementScopeOp::IsEmpty | ElementScopeOp::IsNotEmpty
                    ))
            {
                return Err(format!(
                    "relation selector clause {index} requires no field and an is-empty/is-not-empty operator"
                ));
            }
            if clause.field == Some(ElementField::ConstructionTypeName) {
                return Err(format!(
                    "construction type name is not supported in selector clause {index}"
                ));
            }
            if clause.field == Some(ElementField::TypeDesignation)
                && !matches!(
                    clause.op,
                    ElementScopeOp::Contains(_) | ElementScopeOp::NoneOf(_)
                )
            {
                return Err(format!(
                    "type designation selector clause {index} requires contains or none-of"
                ));
            }
            if clause.field == Some(ElementField::PredefinedType) {
                return Err(format!(
                    "predefined type is not supported in selector clause {index}"
                ));
            }
            if clause.field == Some(ElementField::ProjectPhase) {
                return Err(format!(
                    "project phase is not supported in selector clause {index}"
                ));
            }
            if clause.field == Some(ElementField::MaterialName) {
                return Err(format!(
                    "material name is not supported in selector clause {index}"
                ));
            }
            if clause
                .field
                .as_ref()
                .is_some_and(ElementField::is_global_location)
            {
                return Err(format!(
                    "global location is not supported in selector clause {index}"
                ));
            }
            match (&clause.field, &clause.op) {
                (Some(ElementField::ModelDomain), ElementScopeOp::DomainOneOf(targets)) => {
                    if targets.is_empty() {
                        return Err(format!(
                            "model-domain selector clause {index} has no targets"
                        ));
                    }
                }
                (Some(ElementField::ModelDomain), _) => {
                    return Err(format!(
                        "model-domain selector clause {index} requires a typed domain operator"
                    ));
                }
                (Some(ElementField::BoundingBoxHeight), ElementScopeOp::AtMost(0.0)) => {}
                (Some(ElementField::BoundingBoxHeight), _) => {
                    return Err(format!(
                        "bounding-box height selector clause {index} requires at-most numeric zero"
                    ));
                }
                (_, ElementScopeOp::AtMost(_)) => {
                    return Err(format!(
                        "numeric at-most selector clause {index} requires bounding-box height"
                    ));
                }
                (_, ElementScopeOp::DomainOneOf(_)) => {
                    return Err(format!(
                        "typed domain operator on selector clause {index} requires the model-domain field"
                    ));
                }
                (_, ElementScopeOp::Contains(targets)) if targets.is_empty() => {
                    return Err(format!("contains selector clause {index} has no targets"));
                }
                _ => {}
            }
        }
        for (index, requirement) in requirements.iter().enumerate() {
            if matches!(
                &requirement.field,
                ElementField::Property { name, .. } if name.trim().is_empty()
            ) {
                return Err(format!(
                    "property requirement {index} has an empty property name"
                ));
            }
            let unary = matches!(
                requirement.operator,
                PropertyPredicateOp::IsUndefined
                    | PropertyPredicateOp::IsDefined
                    | PropertyPredicateOp::IsEmpty
                    | PropertyPredicateOp::IsNotEmpty
            );
            if !unary && requirement.targets.is_empty() {
                return Err(format!("property requirement {index} has no target"));
            }
            if matches!(
                requirement.operator,
                PropertyPredicateOp::AtMost
                    | PropertyPredicateOp::AtLeast
                    | PropertyPredicateOp::Greater
                    | PropertyPredicateOp::Smaller
            ) && (requirement.targets.len() != 1
                || !matches!(
                    requirement.targets[0],
                    PredicateValue::Number(_) | PredicateValue::Integer(_)
                ))
            {
                return Err(format!(
                    "ordered property requirement {index} needs one numeric target"
                ));
            }
            if requirement.field == ElementField::MaterialName
                && (requirement.operator != PropertyPredicateOp::IsDefined
                    || !requirement.targets.is_empty())
            {
                return Err(format!(
                    "material assignment requirement {index} only supports targetless is-defined"
                ));
            }

            if requirement.field == ElementField::Geometry
                && (!matches!(
                    requirement.operator,
                    PropertyPredicateOp::IsDefined | PropertyPredicateOp::IsUndefined
                ) || !requirement.targets.is_empty())
            {
                return Err(format!(
                    "geometry requirement {index} only supports targetless is-defined/is-undefined"
                ));
            }
            if requirement.field == ElementField::GeometryVolume
                && (requirement.operator != PropertyPredicateOp::Smaller
                    || requirement.targets.as_slice() != [PredicateValue::Number(0.0)])
            {
                return Err(format!(
                    "geometry volume requirement {index} only supports smaller than numeric zero"
                ));
            }
            if requirement.field == ElementField::HasPositiveSpaceDoorArea
                && (requirement.applies_to.as_deref() != Some("IFCSPACE")
                    || requirement.operator != PropertyPredicateOp::Equals
                    || requirement.targets.as_slice() != [PredicateValue::Boolean(true)])
            {
                return Err(format!(
                    "positive space door-area requirement {index} only supports IFCSPACE equals true"
                ));
            }
            if requirement.field == ElementField::BoundingBoxHeight {
                return Err(format!(
                    "bounding-box height is only supported in selector clause {index}"
                ));
            }
            if requirement.field.is_global_location()
                && (requirement.operator != PropertyPredicateOp::Equals
                    || requirement.targets.len() != 1
                    || !matches!(requirement.targets[0], PredicateValue::Number(_)))
            {
                return Err(format!(
                    "global location requirement {index} requires equals with one numeric target"
                ));
            }
        }
        for (index, requirement) in non_evaluating_requirements.iter().enumerate() {
            if requirement
                .applies_to
                .as_deref()
                .is_some_and(|ifc_type| ifc_type.trim().is_empty())
            {
                return Err(format!(
                    "non-evaluating requirement {index} has an empty IFC type"
                ));
            }
        }
        Ok(())
    }
}
