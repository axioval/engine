//! Material layers: the layers a wall, slab or roof is built of, as its
//! source states them.
//!
//! A source states an element's layers in its own order, each with its
//! material, thickness and the facts it records of the layer, and where
//! it states one, the direction they stack in: along an axis of the
//! element's placement frame ([`LayerAxis`]), from the first layer towards
//! the last ([`LayerDirection`]). Nothing here decides which face is the
//! inside: that is read from the spaces the element bounds
//! (`measured/layers.rs`), and an element whose source states no
//! direction has none, never one guessed from its geometry or its type.
//!
//! An element without material states no layers (an empty list, exactly).
//! A material that is not a layer set (a single material, a constituent,
//! profile or list) is refused by name ([`MaterialLayerError::NotLayered`]):
//! its parts have no order or thickness, and are never read as layers.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId, SourceId};
use thiserror::Error;

use crate::services::reviewable_exact_evidence;
use crate::{SnapshotBoundService, SourceSnapshot};

/// Failure to read an element's material layers.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MaterialLayerError {
    /// The service does not cover the object's source.
    #[error("material-layer service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The object is not part of the source.
    #[error("object `{0}` is not in the source")]
    UnknownObject(ObjectId),
    /// The element's material is not a layer set: `kind` names what it is
    /// (`constituent-set`, `profile-set`, `material`, `list`).
    #[error("the material of `{object}` is a {kind}, not a layer set, so it has no layers")]
    NotLayered {
        /// The element.
        object: ObjectId,
        /// What its material is.
        kind: String,
    },
    /// Two assignments the source cannot tell apart.
    #[error("material layers conflict: {0}")]
    Conflicting(String),
    /// The material is malformed or cannot be read.
    #[error("material layers cannot be read: {0}")]
    Unreadable(String),
    /// A thickness is negative or not finite, or a layer states no
    /// identity.
    #[error("material layers are invalid: {0}")]
    Invalid(String),
    /// The evidence is not exact, not reviewable, or from another source.
    #[error("material-layer evidence is not exact and reviewable")]
    InexactEvidence,
    /// The answer is about another object.
    #[error("material-layer answer is about another object")]
    ResponseRequestMismatch,
}

/// An axis of the element's placement frame (`MetricFrame`'s right,
/// forward and up).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerAxis {
    /// The first axis (`right`; IFC `AXIS1`).
    First,
    /// The second axis (`forward`; IFC `AXIS2`, a wall's thickness).
    Second,
    /// The third axis (`up`; IFC `AXIS3`, a slab's thickness).
    Third,
}

/// Which way along its axis the layers stack, from the first to the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerSense {
    /// Along the axis.
    Positive,
    /// Against the axis.
    Negative,
}

/// The direction the layers stack in, from the first towards the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerDirection {
    /// The axis of the element's frame they stack along.
    pub axis: LayerAxis,
    /// Along it or against it.
    pub sense: LayerSense,
}

/// One layer as the source states it.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialLayer {
    /// The layer's identity in its source (`#32`).
    pub locator: String,
    /// The name of its material; `None` where it states none (an air gap).
    pub material: Option<String>,
    /// Its own name; `None` where it states none.
    pub name: Option<String>,
    /// Its category (its role in the set); `None` where it states none.
    pub category: Option<String>,
    /// Its thickness in metres, or why it cannot be read (a unit the
    /// source does not resolve), which refuses that measure alone.
    pub thickness: Result<f64, String>,
    /// Whether it is a ventilated air gap; `None` where the source states
    /// neither, or that it is unknown.
    pub ventilated: Option<bool>,
    /// Its priority at connections; `None` where it states none.
    pub priority: Option<i64>,
}

/// An element's material layers, in the source's order.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialLayers {
    object: ObjectId,
    layers: Vec<MaterialLayer>,
    direction: Option<LayerDirection>,
    evidence: Evidence,
}

impl MaterialLayers {
    /// Layers of `object` in the source's order, the direction they stack
    /// in where the source states one, and the exact, reviewable evidence
    /// of the object's source the statement comes from.
    ///
    /// # Errors
    ///
    /// [`MaterialLayerError::InexactEvidence`] for evidence that is not
    /// exact, reviewable or of the object's source: layers are stated,
    /// never estimated. [`MaterialLayerError::Invalid`] for a thickness
    /// that is negative or not finite, or a layer without an identity.
    pub fn try_new(
        object: ObjectId,
        layers: Vec<MaterialLayer>,
        direction: Option<LayerDirection>,
        evidence: Evidence,
    ) -> Result<Self, MaterialLayerError> {
        if !reviewable_exact_evidence(&evidence) || evidence.source != object.source {
            return Err(MaterialLayerError::InexactEvidence);
        }
        for layer in &layers {
            if layer.locator.trim().is_empty() {
                return Err(MaterialLayerError::Invalid(format!(
                    "a layer of {object} states no identity"
                )));
            }
            if let Ok(thickness) = layer.thickness
                && !(thickness.is_finite() && thickness >= 0.0)
            {
                return Err(MaterialLayerError::Invalid(format!(
                    "layer {} of {object} is {thickness} m thick",
                    layer.locator
                )));
            }
        }
        Ok(Self {
            object,
            layers,
            direction,
            evidence,
        })
    }

    /// The element.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// Its layers in the source's order; none where it has no material.
    #[must_use]
    pub fn layers(&self) -> &[MaterialLayer] {
        &self.layers
    }

    /// The direction they stack in; `None` where the source states none.
    #[must_use]
    pub fn direction(&self) -> Option<LayerDirection> {
        self.direction
    }

    /// Where the statement comes from.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Trusted adapter seam stating elements' material layers.
pub trait MaterialLayerService: Send + Sync + 'static {
    /// Exact source snapshots this service answers for.
    // gate: reads
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// The material layers of `object` as its source states them.
    // gate: reads
    fn material_layers(&self, object: &ObjectId) -> Result<MaterialLayers, MaterialLayerError>;
}

/// Registry handle for a [`MaterialLayerService`].
#[derive(Clone)]
pub struct MaterialLayerServiceHandle(Arc<dyn MaterialLayerService>);

impl MaterialLayerServiceHandle {
    /// Wraps a trusted material-layer service.
    #[must_use]
    pub fn new(service: Arc<dyn MaterialLayerService>) -> Self {
        Self(service)
    }

    /// The layers of `object`. Objects of an uncovered source are refused,
    /// and so are layers of another object.
    pub fn material_layers(&self, object: &ObjectId) -> Result<MaterialLayers, MaterialLayerError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| *snapshot.source() == object.source)
        {
            return Err(MaterialLayerError::UncoveredSource(object.source.clone()));
        }
        let layers = crate::timing::host("material-layers", || self.0.material_layers(object))?;
        if layers.object() != object {
            return Err(MaterialLayerError::ResponseRequestMismatch);
        }
        Ok(layers)
    }
}

impl SnapshotBoundService for MaterialLayerServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn layer(locator: &str, thickness: f64) -> MaterialLayer {
        MaterialLayer {
            locator: locator.into(),
            material: Some("ex-board".into()),
            name: None,
            category: None,
            thickness: Ok(thickness),
            ventilated: None,
            priority: None,
        }
    }

    struct Fixed(Vec<SourceSnapshot>, ObjectId);

    impl MaterialLayerService for Fixed {
        fn source_snapshots(&self) -> &[SourceSnapshot] {
            &self.0
        }
        fn material_layers(&self, _: &ObjectId) -> Result<MaterialLayers, MaterialLayerError> {
            MaterialLayers::try_new(
                self.1.clone(),
                vec![layer("#2", 0.1)],
                None,
                Evidence::exact(source(), "layers:#1"),
            )
        }
    }

    #[test]
    fn layers_are_stated_exactly_and_never_negative() {
        let exact = Evidence::exact(source(), "layers:#1");
        assert!(
            MaterialLayers::try_new(id("#1"), vec![layer("#2", 0.1)], None, exact.clone()).is_ok()
        );
        let mut approximate = exact.clone();
        approximate.exact = false;
        assert_eq!(
            MaterialLayers::try_new(id("#1"), vec![], None, approximate),
            Err(MaterialLayerError::InexactEvidence)
        );
        assert!(matches!(
            MaterialLayers::try_new(id("#1"), vec![layer("#2", -0.1)], None, exact.clone()),
            Err(MaterialLayerError::Invalid(_))
        ));
        assert!(matches!(
            MaterialLayers::try_new(id("#1"), vec![layer(" ", 0.1)], None, exact.clone()),
            Err(MaterialLayerError::Invalid(_))
        ));
        // A thickness whose unit cannot be resolved is refused alone.
        let mut unread = layer("#2", 0.0);
        unread.thickness = Err("no length unit".into());
        assert!(MaterialLayers::try_new(id("#1"), vec![unread], None, exact).is_ok());
    }

    #[test]
    fn the_handle_refuses_other_sources_and_other_objects() {
        let snapshot = SourceSnapshot::try_new(source(), "r1", "sha256:x").unwrap();
        let handle = MaterialLayerServiceHandle::new(Arc::new(Fixed(vec![snapshot], id("#1"))));
        assert!(handle.material_layers(&id("#1")).is_ok());
        assert_eq!(
            handle.material_layers(&id("#9")),
            Err(MaterialLayerError::ResponseRequestMismatch)
        );
        let other = ObjectId::new(SourceId::new("cad", "n").unwrap(), "#1").unwrap();
        assert!(matches!(
            handle.material_layers(&other),
            Err(MaterialLayerError::UncoveredSource(_))
        ));
    }
}
