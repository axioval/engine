//! The face a surface value reads, and the pieces of a face one by one.
//!
//! `face=top` and `face=bottom` read the normals the vertical-extent
//! service certifies for a face. `face=facing;direction=…;tolerance=…`
//! reads the pieces of a closed body's whole boundary
//! ([`FacePieceSet::Boundary`]) whose outward normal lies within the
//! tolerance of the direction: a piece faces it when every part's normal
//! surely does, and does not when some part's surely does not. Whether a
//! piece faces it is decided with the expression language's sound
//! interval arithmetic over the normal boxes, never from a midpoint.
//!
//! The member list `face_pieces` ([`FacePieceMeasures`]) lists the pieces
//! of the face, each with its slope, area and direction of descent. A
//! piece whose facing cannot be decided is a possible member, so an
//! aggregate over the list stays undecided rather than dropping it.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use axioval_ir::measured::{FACE_PIECES, FACING, MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, MEASURED_SET, ObjectId, QuantityDimension};

use super::provider::{MeasuredMember, MeasuredProvider, Measurement, MemberValue};
use super::surface;
use crate::RuleContext;
use crate::expression::Interval;
use crate::properties::PropertyResolutionError;
use crate::vertical_extent::{
    FaceNormal, FacePiece, FacePieceSet, SurfaceFace, VerticalExtentServiceHandle,
};

/// The pieces facing a direction within a tolerance.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Facing {
    direction: [f64; 3],
    /// The cosine of the tolerance: a normal faces the direction when the
    /// cosine of its angle to it is at least this.
    least_cosine: Interval,
}

/// The face a call reads.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Selection {
    Face(SurfaceFace),
    Facing(Facing),
}

/// Whether a piece faces a direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Faces {
    Surely,
    Never,
    Undecided,
}

impl Selection {
    /// The face `call` reads, as the registry validated it.
    fn of(call: &MeasuredCall) -> Result<Self, String> {
        match call.choice("face") {
            Some("bottom") => Ok(Self::Face(SurfaceFace::Bottom)),
            Some(FACING) => {
                let (
                    Some(MeasuredArgument::Vector(direction)),
                    Some(MeasuredArgument::Length(degrees)),
                ) = (call.argument("direction"), call.argument("tolerance"))
                else {
                    return Err("`face=facing` needs `direction` and `tolerance`".into());
                };
                // The tolerance in radians, π held between its two nearest
                // doubles.
                let pi = Interval {
                    lower: PI,
                    upper: PI.next_up(),
                };
                let tolerance = Interval::point(*degrees)
                    .times(pi)
                    .and_then(|product| product.divided_by(Interval::point(180.0)))
                    .map_err(|_| "the tolerance cannot be measured".to_owned())?;
                Ok(Self::Facing(Facing {
                    direction: *direction,
                    least_cosine: tolerance.cos(),
                }))
            }
            _ => Ok(Self::Face(SurfaceFace::Top)),
        }
    }

    /// The set of pieces a piece measurement reads.
    fn set(self) -> FacePieceSet {
        match self {
            Self::Face(face) => face.into(),
            Self::Facing(_) => FacePieceSet::Boundary,
        }
    }

    /// Whether `piece` belongs to the face.
    fn holds(self, piece: &FacePiece) -> Faces {
        match self {
            Self::Face(_) => Faces::Surely,
            Self::Facing(facing) => facing.faces(piece),
        }
    }
}

fn square(value: Interval) -> Option<Interval> {
    let magnitude = value.abs();
    magnitude.times(magnitude).ok()
}

impl Facing {
    /// The cosine of the angle between `normal` and the direction.
    fn cosine(&self, normal: &FaceNormal) -> Option<Interval> {
        let (lower, upper) = (normal.lower(), normal.upper());
        let mut dot = Interval::point(0.0);
        let mut length = Interval::point(0.0);
        for axis in 0..3 {
            let component = Interval {
                lower: lower[axis],
                upper: upper[axis],
            };
            dot = dot
                .plus(
                    component
                        .times(Interval::point(self.direction[axis]))
                        .ok()?,
                )
                .ok()?;
            length = length.plus(square(component)?).ok()?;
        }
        let along: f64 = self.direction.iter().map(|c| c * c).sum();
        let scale = length
            .sqrt()
            .ok()?
            .times(Interval::point(along).sqrt().ok()?)
            .ok()?;
        let cosine = dot.divided_by(scale).ok()?;
        Some(Interval {
            lower: cosine.lower.clamp(-1.0, 1.0),
            upper: cosine.upper.clamp(-1.0, 1.0),
        })
    }

    /// Whether every part of `piece` surely faces the direction (`Surely`),
    /// some part surely does not (`Never`), or neither is sure.
    fn faces(&self, piece: &FacePiece) -> Faces {
        let mut sure = true;
        for normal in piece.normals() {
            match self.cosine(normal) {
                Some(cosine) if cosine.upper < self.least_cosine.lower => return Faces::Never,
                Some(cosine) if cosine.lower >= self.least_cosine.upper => {}
                _ => sure = false,
            }
        }
        if sure {
            Faces::Surely
        } else {
            Faces::Undecided
        }
    }
}

/// The normals of the face `call` reads of `object`, with the evidence of
/// their measurement: for `top` and `bottom` the face's own, for `facing`
/// those of every piece facing the direction. A piece whose facing cannot
/// be decided, or no piece facing it, leaves the face unmeasured.
pub(crate) fn face_normals(
    service: &VerticalExtentServiceHandle,
    call: &MeasuredCall,
    object: &ObjectId,
) -> Result<(Vec<FaceNormal>, Evidence), String> {
    match Selection::of(call)? {
        Selection::Face(face) => {
            let normals = service
                .measure_face_normals(object, face)
                .map_err(|error| error.to_string())?;
            Ok((normals.normals().to_vec(), normals.evidence().clone()))
        }
        selection @ Selection::Facing(_) => {
            let pieces = service
                .measure_face_pieces(object, selection.set())
                .map_err(|error| error.to_string())?;
            let mut normals = Vec::new();
            for piece in pieces.pieces() {
                match selection.holds(piece) {
                    Faces::Surely => normals.extend_from_slice(piece.normals()),
                    Faces::Never => {}
                    Faces::Undecided => {
                        return Err("whether a piece faces the direction cannot be decided".into());
                    }
                }
            }
            if normals.is_empty() {
                return Err("the body has no piece facing the direction".into());
            }
            Ok((normals, pieces.evidence().clone()))
        }
    }
}

/// Measures the member list `face_pieces`: the pieces of a face, each with
/// its `slope`, `area` and `gradient_direction`, over the vertical-extent
/// service.
#[derive(Clone, Copy, Debug, Default)]
pub struct FacePieceMeasures;

impl FacePieceMeasures {
    fn pieces(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let refused = |why: String| {
            PropertyResolutionError::Unavailable(format!(
                "`{MEASURED_SET}` member list `{FACE_PIECES}` of {object}: {why}"
            ))
        };
        let service = context
            .services
            .get::<VerticalExtentServiceHandle>()
            .ok_or_else(|| {
                PropertyResolutionError::MissingService(format!(
                    "no vertical-extent service is registered, so `{FACE_PIECES}` cannot be \
                     measured"
                ))
            })?;
        let selection = Selection::of(call).map_err(refused)?;
        let pieces = service
            .measure_face_pieces(object, selection.set())
            .map_err(|error| refused(error.to_string()))?;
        let exact = pieces.evidence().exact;
        let locator = &pieces.evidence().locator;
        let mut members = Vec::new();
        for (index, piece) in pieces.pieces().iter().enumerate() {
            let certain = match selection.holds(piece) {
                Faces::Surely => true,
                Faces::Undecided => false,
                Faces::Never => continue,
            };
            let at = |field: &str| format!("{locator}#piece={}:{field}", index + 1);
            let angle = |value: Interval, field: &str| {
                MemberValue::Measured(Measurement::Value {
                    lower: value.lower,
                    upper: value.upper,
                    dimension: Some(QuantityDimension::PlaneAngle),
                    locator: at(field),
                })
            };
            let slope = match surface::piece_slope(piece.normals()) {
                Ok(slope) => angle(slope, "slope"),
                Err(why) => MemberValue::Undecided { why },
            };
            let direction = match surface::gradient_direction(piece.normals()) {
                Ok(bearing) => angle(bearing, "gradient_direction"),
                Err(_) if surface::exactly_level(piece.normals()) => {
                    MemberValue::Measured(Measurement::Absent {
                        locator: at("gradient_direction:level"),
                    })
                }
                Err(why) => MemberValue::Undecided { why },
            };
            let (lower, upper) = piece.area_square_metres();
            let area = MemberValue::Measured(Measurement::Value {
                lower,
                upper,
                dimension: Some(QuantityDimension::Area),
                locator: at("area"),
            });
            members.push(MeasuredMember {
                certain,
                exact,
                fields: BTreeMap::from([
                    ("slope", slope),
                    ("area", area),
                    ("gradient_direction", direction),
                ]),
            });
        }
        Ok((members, vec![pieces.evidence().clone()]))
    }
}

impl MeasuredProvider for FacePieceMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[FACE_PIECES]
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        Self::pieces(call, object, context).map(|(members, _)| members)
    }

    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        Self::pieces(call, object, context)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(vector: [f64; 3]) -> FaceNormal {
        FaceNormal::exact(vector).unwrap()
    }

    fn facing(direction: [f64; 3], degrees: f64) -> Selection {
        let call = axioval_ir::measured::parse(&format!(
            "slope;face=facing;direction={},{},{};tolerance={degrees}",
            direction[0], direction[1], direction[2]
        ))
        .unwrap();
        Selection::of(&call).unwrap()
    }

    fn piece(normals: &[[f64; 3]]) -> FacePiece {
        FacePiece::try_new(normals.iter().map(|n| exact(*n)).collect(), 1.0, 1.0).unwrap()
    }

    #[test]
    fn a_piece_faces_a_direction_within_the_tolerance() {
        // A batter falling 1:1.5 to the east leans about 56.3° from up.
        let east_batter = piece(&[[1.0, 0.0, 1.5]]);
        let crest = piece(&[[0.0, 0.0, 1.0]]);
        let east = facing([1.0, 0.0, 0.0], 60.0);
        assert_eq!(east.holds(&east_batter), Faces::Surely);
        assert_eq!(east.holds(&crest), Faces::Never);
        let up = facing([0.0, 0.0, 1.0], 10.0);
        assert_eq!(up.holds(&crest), Faces::Surely);
        assert_eq!(up.holds(&east_batter), Faces::Never);
        // A piece one part of which faces and another surely does not is
        // not facing.
        assert_eq!(
            east.holds(&piece(&[[1.0, 0.0, 1.5], [0.0, 0.0, 1.0]])),
            Faces::Never
        );
        // Everything faces within 180°.
        assert_eq!(facing([0.0, 0.0, 1.0], 180.0).holds(&crest), Faces::Surely);
    }

    #[test]
    fn a_normal_on_the_tolerance_is_undecided() {
        // Exactly 45° from east: the rounding of the cosines decides
        // nothing either way.
        let edge = piece(&[[1.0, 0.0, 1.0]]);
        assert_eq!(facing([1.0, 0.0, 0.0], 45.0).holds(&edge), Faces::Undecided);
        // A box straddling the cone is undecided too.
        let wide = FacePiece::try_new(
            vec![FaceNormal::try_new([0.9, 0.0, 0.5], [1.1, 0.0, 1.5]).unwrap()],
            1.0,
            1.0,
        )
        .unwrap();
        assert_eq!(facing([1.0, 0.0, 0.0], 50.0).holds(&wide), Faces::Undecided);
    }
}
