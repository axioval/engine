//! The member list `layers` ([`MaterialLayerMeasures`]): an element's
//! material layers as its source states them ([`crate::material_layers`]),
//! inside first where the inside is decided.
//!
//! The inside is read from the spaces the element bounds, never guessed:
//! the `spaces` kinds name them, geometry places each beside a face of the
//! element (`axioval:derived.adjacent-across`, asked of the derived
//! service directly, which records the face and its normal), and the inside
//! is the face beside which spaces lie when none lies beside the other. The
//! layers stack along the direction their source states, an axis of the
//! element's placement frame: where it runs away from the inner face the
//! first layer is the innermost, where it runs towards it the last, and
//! where it runs along the faces (within 45° of them) nothing is decided.
//! Whatever cannot be read (no kinds, no stated direction, no frame, no
//! adjacency, spaces beside both faces or neither) leaves each layer's
//! `position` and `from_inside` undecided with the reason, and the layers
//! listed in the source's order; every field the source states stands.

use std::collections::BTreeMap;

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, MEASURED_SET, ObjectId, QuantityDimension};

use super::provider::{MeasuredMember, MeasuredProvider, Measurement, MemberValue};
use super::{bounds, span};
use crate::RuleContext;
use crate::derived_relationships::{Derivation, DerivedRelationshipServiceHandle, across_face};
use crate::material_layers::{
    LayerAxis, LayerSense, MaterialLayer, MaterialLayerError, MaterialLayerServiceHandle,
    MaterialLayers,
};
use crate::properties::PropertyResolutionError;
use crate::relationships::{
    RelationshipQuery, RelationshipSelectionRequest, SemanticRelationship, TraversalDirection,
};
use crate::{AdjacentSide, ObjectFrameServiceHandle, TypeHierarchyServiceHandle};

/// The member list measured here.
const LAYERS: &str = "layers";

/// The least cosine between the layers' direction and the inner face's
/// normal that decides which layer lies inside: within 45° of the normal.
const ALONG_NORMAL: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Measures the member list `layers` over the material-layer service.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaterialLayerMeasures;

/// Which layer lies inside, as decided from the spaces beside the faces.
struct Inside {
    /// Whether the last layer the source states is the innermost.
    reversed: bool,
    /// Whether everything the decision rests on is exact.
    exact: bool,
    /// What it rests on: the frame and the adjacency.
    evidence: Vec<Evidence>,
}

impl MaterialLayerMeasures {
    #[allow(clippy::too_many_lines)] // One field after another.
    fn layers(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let service = context
            .services
            .get::<MaterialLayerServiceHandle>()
            .ok_or_else(|| {
                PropertyResolutionError::MissingService(format!(
                    "no material-layer service is registered, so `{LAYERS}` cannot be read"
                ))
            })?;
        let read = service
            .material_layers(object)
            .map_err(|error| match error {
                MaterialLayerError::InexactEvidence
                | MaterialLayerError::Invalid(_)
                | MaterialLayerError::ResponseRequestMismatch => {
                    PropertyResolutionError::InvalidValue
                }
                MaterialLayerError::Conflicting(why) => PropertyResolutionError::Conflicting(
                    format!("`{MEASURED_SET}` member list `{LAYERS}` of {object}: {why}"),
                ),
                other => PropertyResolutionError::Unavailable(format!(
                    "`{MEASURED_SET}` member list `{LAYERS}` of {object}: {other}"
                )),
            })?;
        let mut evidence = vec![read.evidence().clone()];
        let inside = if read.layers().is_empty() {
            Err("the element states no layers".to_owned())
        } else {
            Self::inside(call, object, &read, context)
        };
        let order: Vec<usize> = match &inside {
            Ok(inside) if inside.reversed => (0..read.layers().len()).rev().collect(),
            _ => (0..read.layers().len()).collect(),
        };
        if let Ok(inside) = &inside {
            evidence.extend(inside.evidence.iter().cloned());
        }
        let exact = read.evidence().exact;
        let list = &read.evidence().locator;
        let length = Some(QuantityDimension::Length);
        let mut members = Vec::with_capacity(order.len());
        // The summed thickness of the layers listed so far, from the inner
        // face; `Err` once one of them cannot be read.
        let mut depth: Result<(f64, f64), String> = Ok((0.0, 0.0));
        for (place, &index) in order.iter().enumerate() {
            let layer = &read.layers()[index];
            let at = |field: &str| format!("{list}#layer={}:{field}", layer.locator);
            let null =
                |field: &str| MemberValue::Measured(Measurement::Absent { locator: at(field) });
            let text = |value: &Option<String>, field: &str| match value {
                Some(text) => MemberValue::Text { text: text.clone() },
                None => null(field),
            };
            let point = |value: f64, dimension, field: &str| {
                MemberValue::Measured(Measurement::Rounded {
                    lower: value,
                    upper: value,
                    dimension,
                    locator: at(field),
                })
            };
            let thickness = match &layer.thickness {
                Ok(value) => point(*value, length, "thickness"),
                Err(why) => MemberValue::Undecided {
                    why: format!("the thickness of layer {}: {why}", layer.locator),
                },
            };
            let (position, from_inside) = match &inside {
                Ok(decided) => {
                    #[allow(clippy::cast_precision_loss)] // A layer count.
                    let rank = (place + 1) as f64;
                    let cited = |(lower, upper): (f64, f64), dimension, field: &str| {
                        MemberValue::Measured(Measurement::Cited {
                            lower,
                            upper,
                            dimension,
                            locator: at(field),
                            exact: decided.exact,
                        })
                    };
                    let from_inside = match &depth {
                        Ok(range) => cited(*range, length, "from_inside"),
                        Err(why) => MemberValue::Undecided { why: why.clone() },
                    };
                    (cited((rank, rank), None, "position"), from_inside)
                }
                Err(why) => {
                    let why = format!("the inside of {object} is not decided: {why}");
                    (
                        MemberValue::Undecided { why: why.clone() },
                        MemberValue::Undecided { why },
                    )
                }
            };
            depth = match (depth, &layer.thickness) {
                (Ok((lower, upper)), Ok(thickness)) => {
                    bounds(span(lower, upper).plus(span(*thickness, *thickness)))
                        .map_err(|_| format!("the layers of {object} overflow"))
                }
                (Ok(_), Err(why)) => Err(format!(
                    "the thickness of layer {} cannot be read: {why}",
                    layer.locator
                )),
                (Err(why), _) => Err(why),
            };
            #[allow(clippy::cast_precision_loss)] // A layer count.
            let stated = (index + 1) as f64;
            members.push(MeasuredMember {
                certain: true,
                exact,
                fields: BTreeMap::from([
                    ("layer", MemberValue::Text { text: words(layer) }),
                    ("index", point(stated, None, "index")),
                    ("position", position),
                    ("material", text(&layer.material, "material")),
                    ("layer_name", text(&layer.name, "layer_name")),
                    ("layer_category", text(&layer.category, "layer_category")),
                    ("thickness", thickness),
                    ("from_inside", from_inside),
                    (
                        "ventilated",
                        match layer.ventilated {
                            Some(value) => MemberValue::Truth {
                                value,
                                locator: at("ventilated"),
                            },
                            None => null("ventilated"),
                        },
                    ),
                    (
                        "priority",
                        match layer.priority {
                            #[allow(clippy::cast_precision_loss)] // 0 to 100.
                            Some(value) => point(value as f64, None, "priority"),
                            None => null("priority"),
                        },
                    ),
                ]),
                evidence: Vec::new(),
            });
        }
        Ok((members, evidence))
    }

    /// Which layer lies inside, or why that is not decided.
    fn inside(
        call: &MeasuredCall,
        object: &ObjectId,
        read: &MaterialLayers,
        context: &RuleContext<'_>,
    ) -> Result<Inside, String> {
        let Some(MeasuredArgument::SourceKind(kinds)) = call.argument("spaces") else {
            return Err("no space kinds are given (`spaces`)".into());
        };
        let direction = read
            .direction()
            .ok_or("its source states no direction for the layers")?;
        let frames = context
            .services
            .get::<ObjectFrameServiceHandle>()
            .ok_or("no object-frame service is registered")?;
        let frame = frames
            .object_frame(object)
            .map_err(|error| format!("its frame cannot be read: {error}"))?;
        let axis = match direction.axis {
            LayerAxis::First => frame.frame().right(),
            LayerAxis::Second => frame.frame().forward(),
            LayerAxis::Third => frame.frame().up(),
        }
        .components();
        let sense = match direction.sense {
            LayerSense::Positive => 1.0,
            LayerSense::Negative => -1.0,
        };
        let spaces = spaces_of(kinds, object, context)?;
        if spaces.is_empty() {
            return Err(format!("no object of the kinds `{kinds}` is in its source"));
        }
        let derived = context
            .services
            .get::<DerivedRelationshipServiceHandle>()
            .ok_or("no geometry derives the spaces beside its faces")?;
        let identity = Derivation::AdjacentAcross {
            tolerance_metres: 0.05,
            overlap_metres: 0.3,
        }
        .to_string();
        let relationship = SemanticRelationship::try_new(identity).map_err(|e| e.to_string())?;
        let request = RelationshipSelectionRequest::try_new(
            object.clone(),
            spaces,
            RelationshipQuery::Related {
                relationship,
                direction: TraversalDirection::Forward,
                follow_chain: false,
            },
        )
        .map_err(|error| error.to_string())?;
        let selection = derived
            .select(&request)
            .map_err(|error| format!("the spaces beside its faces cannot be derived: {error}"))?;
        let mut inner: Option<[f64; 3]> = None;
        let mut faces = [false, false];
        for space in selection.candidates() {
            let mut placed = false;
            for item in selection.evidence() {
                if let Some((side, normal)) = across_face(&item.locator, object, space) {
                    placed = true;
                    faces[usize::from(side == AdjacentSide::Negative)] = true;
                    inner = Some(normal);
                }
            }
            if !placed {
                return Err(format!(
                    "the adjacency records no face for space {space} beside it"
                ));
            }
        }
        let normal = match (faces, inner) {
            ([true, true], _) => {
                return Err("spaces of the kinds lie beside both of its faces".into());
            }
            (_, Some(normal)) => normal,
            (_, None) => return Err("no space of the kinds lies beside either face".into()),
        };
        let cosine = sense * (axis[0] * normal[0] + axis[1] * normal[1] + axis[2] * normal[2]);
        if cosine.abs() < ALONG_NORMAL {
            return Err(format!(
                "its layers run along its faces (the cosine between their direction and the \
                 inner face's normal is {cosine:.3})"
            ));
        }
        let mut evidence = vec![frame.evidence().clone()];
        evidence.extend(selection.evidence().iter().cloned());
        Ok(Inside {
            // Stacking towards the inner face puts the last layer inside.
            reversed: cosine > 0.0,
            exact: evidence.iter().all(|item| item.exact),
            evidence,
        })
    }
}

/// How a finding names a layer: its identity and its material.
fn words(layer: &MaterialLayer) -> String {
    match &layer.material {
        Some(material) => format!("layer {} ({material})", layer.locator),
        None => format!("layer {}", layer.locator),
    }
}

/// The objects of the source kinds `kinds` in `object`'s source, subtypes
/// through the source's type hierarchy, sorted.
fn spaces_of(
    kinds: &str,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<ObjectId>, String> {
    let kinds: Vec<&str> = kinds.split(',').map(str::trim).collect();
    let hierarchy = context.services.get::<TypeHierarchyServiceHandle>();
    let mut found = Vec::new();
    for candidate in context.project.objects() {
        if candidate.id == *object || candidate.id.source != object.source {
            continue;
        }
        let held = candidate.kind();
        for kind in &kinds {
            let matched =
                held.eq_ignore_ascii_case(kind)
                    || match hierarchy {
                        Some(hierarchy) => hierarchy
                            .is_a(&candidate.id.source, held, kind)
                            .map_err(|error| {
                                format!("whether {} is a {kind}: {error}", candidate.id)
                            })?,
                        None => false,
                    };
            if matched {
                found.push(candidate.id.clone());
                break;
            }
        }
    }
    found.sort();
    Ok(found)
}

impl MeasuredProvider for MaterialLayerMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[LAYERS]
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        Self::layers(call, object, context).map(|(members, _)| members)
    }

    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        Self::layers(call, object, context)
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::MissingService(format!(
            "`{}` of {object} is a member list, not a measured value",
            call.name()
        )))
    }
}
