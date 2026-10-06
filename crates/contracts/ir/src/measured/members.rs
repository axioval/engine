//! Measured member lists: the parts of an object built-in code measures
//! one by one (a flight's steps, a ramp's runs), which an aggregate ranges
//! over and whose fields its expressions read in [`MEMBER_SET`](crate::MEMBER_SET).
//!
//! A list is written like a measured value, `name[;key=value…]`, and
//! parsed against [`MEASURED_MEMBERS`] with [`parse_members`].

use serde::Serialize;

use super::registry::{
    ANGLE_TOLERANCE, FACE, FACE_AXES, FACING, MEMBER_PATH, NO_FACE, NO_GEOMETRY, OPENINGS_MINIMUM,
    PAIRED, SPACED_MEMBERS, en_de,
};
use super::{
    FACE_PIECES, LocalizedText, MeasuredCall, MeasuredDescriptor, MeasuredError, MeasuredExactness,
    MeasuredParameter, MeasuredParameterKind,
};
use crate::QuantityDimension;

mod walking;

/// One list of measured members.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDescriptor {
    /// The list's name, parameters, services, exactness and labels; its
    /// `dimension` is `None`.
    #[serde(flatten)]
    pub list: MeasuredDescriptor,
    /// The fields every member states, in documentation order.
    pub fields: &'static [MemberField],
}

impl MemberDescriptor {
    /// The field `name`, if members state it.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&'static MemberField> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// One field of a measured member, read in [`MEMBER_SET`](crate::MEMBER_SET).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberField {
    pub name: &'static str,
    pub kind: MemberFieldKind,
    /// A short name for editors.
    pub label: &'static [LocalizedText],
    /// What it states, and when it is `null`.
    pub help: &'static [LocalizedText],
}

/// What a member field holds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MemberFieldKind {
    /// A number in the coherent SI unit of `dimension`, an interval when
    /// measured inexactly; `None` for a plain number.
    Number {
        #[serde(skip_serializing_if = "Option::is_none")]
        dimension: Option<QuantityDimension>,
    },
    /// A truth.
    Truth,
    /// Words naming the member, as messages name it; read as text.
    Text,
    /// Objects the member was measured against or found; read as the text
    /// of their identities, joined by `, `.
    Objects,
}

const FLIGHTS: &[&str] = &["walking-surface"];

pub(super) const WALKING_LINE_OFFSET: MeasuredParameter = MeasuredParameter {
    key: "walking_line_offset",
    kind: MeasuredParameterKind::Length { minimum: 0.0 },
    required: false,
    default: None,
    help: &en_de(
        "Where a turning flight is walked: this far from the side it turns towards; \
         without it, midway across its treads.",
        "Wo ein gewendelter Lauf begangen wird: so weit von der Seite, zu der er sich \
         wendet; ohne Angabe mittig über die Stufen.",
    ),
};

const LENGTH: MemberFieldKind = MemberFieldKind::Number {
    dimension: Some(QuantityDimension::Length),
};
const ANGLE: MemberFieldKind = MemberFieldKind::Number {
    dimension: Some(QuantityDimension::PlaneAngle),
};
const RATIO: MemberFieldKind = MemberFieldKind::Number { dimension: None };
const AREA: MemberFieldKind = MemberFieldKind::Number {
    dimension: Some(QuantityDimension::Area),
};

const fn required_length(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: true,
        default: None,
        help,
    }
}

const fn optional_length(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help,
    }
}

const fn optional_kinds(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::SourceKind,
        required: false,
        default: None,
        help,
    }
}

const fn field(
    name: &'static str,
    kind: MemberFieldKind,
    label: &'static [LocalizedText],
    help: &'static [LocalizedText],
) -> MemberField {
    MemberField {
        name,
        kind,
        label,
        help,
    }
}

const TRUTH: MemberFieldKind = MemberFieldKind::Truth;

const fn guard_length(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    required_length(key, help)
}

const fn guard_kinds(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::SourceKind,
        required: false,
        default: None,
        help,
    }
}

/// Every list of measured members, sorted by name.
pub static MEASURED_MEMBERS: &[MemberDescriptor] = &[
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "axes_within",
            parameters: &[
                MeasuredParameter {
                    key: "of",
                    kind: MeasuredParameterKind::SourceKind,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The source kinds of the objects measured against, such as \
                         aisles, `,`-separated.",
                        "Die Quellarten der Objekte, gegen die gemessen wird, etwa \
                         Fahrgassen, durch `,` getrennt.",
                    ),
                },
                MeasuredParameter {
                    key: "reach",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: Some("0"),
                    help: &en_de(
                        "How far from the footprint in plan an object counts, in metres; \
                         0, the default, is touching it.",
                        "Wie weit vom Grundriss entfernt ein Objekt zählt, in Metern; 0, \
                         die Vorgabe, ist berührend.",
                    ),
                },
            ],
            dimension: None,
            services: &["plan-span", "proximity", "type-hierarchy"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &[
                "the object has no body the service can measure",
                "an object near it has no readable extent",
            ],
            label: &en_de("Axes within reach", "Achsen in Reichweite"),
            help: &en_de(
                "The objects of the kinds named within reach of the footprint in plan, \
                 each with the angle between the long axes, as `parking-bay` reads a \
                 bay's orientation to its aisles; one only possibly within reach is an \
                 undecided member.",
                "Die Objekte der genannten Arten in Reichweite des Grundrisses, jedes mit \
                 dem Winkel zwischen den Längsachsen, wie `parking-bay` die Ausrichtung \
                 zur Fahrgasse liest; ein nur möglicherweise erreichtes ist ein \
                 unentschiedenes Element.",
            ),
        },
        fields: &[
            field(
                "angle",
                ANGLE,
                &en_de("Angle", "Winkel"),
                &en_de(
                    "The acute angle between the long axes of both footprints' least-area \
                     rectangles, in `[0, π/2]`, as `parking-bay` reads a bay's orientation \
                     to an aisle and whether a neighbour is parallel to it; undecided where \
                     either has no long axis.",
                    "Der spitze Winkel zwischen den Längsachsen der flächenkleinsten \
                     Rechtecke beider Grundrisse, in `[0, π/2]`, wie `parking-bay` die \
                     Ausrichtung zur Fahrgasse liest und ob ein Nachbar parallel steht; \
                     unentschieden, wo eines keine Längsachse hat.",
                ),
            ),
            field(
                "centre_angle",
                ANGLE,
                &en_de("Angle to the centre", "Winkel zur Mitte"),
                &en_de(
                    "The acute angle between the footprint's long axis and the direction \
                     from its centre to the member's centre, in `[0, π/2]`: about `π/2` for \
                     a neighbour beside it, about `0` for one end to end with it, as \
                     `parking-bay` infers a bay's orientation from its neighbours; \
                     undecided where it has no long axis or the centres are too close.",
                    "Der spitze Winkel zwischen der Längsachse des Grundrisses und der \
                     Richtung von seiner Mitte zur Mitte des Elements, in `[0, π/2]`: etwa \
                     `π/2` für einen Nachbarn daneben, etwa `0` für einen dahinter, wie \
                     `parking-bay` die Ausrichtung einer Stellfläche aus ihren Nachbarn \
                     ableitet; unentschieden, wo sie keine Längsachse hat oder die Mitten \
                     zu nah beieinander liegen.",
                ),
            ),
        ],
    },
    walking::CLEAR_WIDTHS,
    walking::CLEARANCES,
    walking::END_SPACES,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "end_walls",
            parameters: &[
                MeasuredParameter {
                    key: "corridor",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the opening to its corridors.",
                        "Die Beziehungsschritte von der Öffnung zu ihren Fluren.",
                    ),
                },
                MeasuredParameter {
                    key: "kinds",
                    kind: MeasuredParameterKind::SourceKind,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The source kinds the corridors must be of, `,`-separated; any without \
                         it.",
                        "Die Quellarten, von denen die Flure sein müssen, durch `,` getrennt; \
                         ohne Angabe jede.",
                    ),
                },
            ],
            dimension: None,
            services: &["plan-span", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &["the wall an end runs into is undecided"],
            label: &en_de("Corridor end walls", "Flurstirnwände"),
            help: &en_de(
                "The walls the ends of the corridors an opening faces run into, each with \
                 the opening's gap to it and the length of it the opening faces.",
                "Die Wände, auf die die Enden der Flure einer Öffnung stoßen, jede mit dem \
                 Abstand der Öffnung zu ihr und der Länge, der die Öffnung gegenübersteht.",
            ),
        },
        fields: &[
            field(
                "gap",
                LENGTH,
                &en_de("Gap", "Abstand"),
                &en_de(
                    "The plan distance from the opening to the wall segment, zero where they \
                     meet.",
                    "Der Grundrissabstand der Öffnung zum Wandabschnitt, null, wo sie sich \
                     berühren.",
                ),
            ),
            field(
                "facing",
                LENGTH,
                &en_de("Facing", "Gegenüber"),
                &en_de(
                    "The length of the wall segment the opening faces.",
                    "Die Länge des Wandabschnitts, dem die Öffnung gegenübersteht.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "exit_pairs",
            parameters: &[
                MeasuredParameter {
                    key: "exits",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the space to its exits.",
                        "Die Beziehungsschritte vom Raum zu seinen Ausgängen.",
                    ),
                },
                MeasuredParameter {
                    key: "kinds",
                    kind: MeasuredParameterKind::SourceKind,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The source kinds the exits must be of, `,`-separated; any without it.",
                        "Die Quellarten, von denen die Ausgänge sein müssen, durch `,` \
                         getrennt; ohne Angabe jede.",
                    ),
                },
                MeasuredParameter {
                    key: "between",
                    kind: MeasuredParameterKind::Choice {
                        options: &["closest", "centres", "farthest"],
                    },
                    required: false,
                    default: Some("closest"),
                    help: &en_de(
                        "Between the exits' closest points, centres or farthest points in plan.",
                        "Zwischen den nächsten Punkten, Mittelpunkten oder fernsten Punkten der \
                         Ausgänge im Grundriss.",
                    ),
                },
            ],
            dimension: None,
            services: &["plan-span", "proximity", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &["a pair's separation cannot be measured"],
            label: &en_de("Exit pairs", "Ausgangspaare"),
            help: &en_de(
                "Every pair of a space's exits, with how far apart they are in plan.",
                "Jedes Paar der Ausgänge eines Raums, mit ihrem Abstand im Grundriss.",
            ),
        },
        fields: &[field(
            "separation",
            LENGTH,
            &en_de("Separation", "Abstand"),
            &en_de(
                "How far apart the two exits are in plan.",
                "Wie weit die beiden Ausgänge im Grundriss auseinanderliegen.",
            ),
        )],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: FACE_PIECES,
            parameters: &[FACE, FACING[0], FACING[1]],
            dimension: None,
            services: &["vertical-extent"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &[NO_GEOMETRY, NO_FACE, "an open surface faces no direction"],
            label: &en_de("Face pieces", "Flächenteile"),
            help: &en_de(
                "The planar or smooth pieces of a face of the body, one by one, in the \
                 order the geometry service lists them. With `face=facing`, a piece \
                 whose facing cannot be decided is a possible member.",
                "Die ebenen oder glatten Teile einer Fläche des Körpers, einzeln, in der \
                 Reihenfolge des Geometriedienstes. Mit `face=facing` ist ein Teil, dessen \
                 Ausrichtung sich nicht entscheiden lässt, ein mögliches Mitglied.",
            ),
        },
        fields: &[
            field(
                "slope",
                ANGLE,
                &en_de("Slope", "Neigung"),
                &en_de(
                    "The piece's steepest gradient as an angle from the horizontal, in \
                     `[0, π/2]`: the hull over the piece, `π/2` where it stands vertical.",
                    "Das steilste Gefälle des Teils als Winkel zur Waagerechten, in \
                     `[0, π/2]`: die Hülle über das Teil, `π/2`, wo es senkrecht steht.",
                ),
            ),
            field(
                "area",
                AREA,
                &en_de("Area", "Fläche"),
                &en_de("The piece's surface area.", "Der Flächeninhalt des Teils."),
            ),
            field(
                "gradient_direction",
                ANGLE,
                &en_de("Direction of descent", "Fallrichtung"),
                &en_de(
                    "The plan bearing of the piece's steepest descent, clockwise from plan \
                     north; `null` for a level piece, undecided where the piece may be level \
                     or stand vertical.",
                    "Die Grundrissrichtung des steilsten Gefälles des Teils, im Uhrzeigersinn \
                     von Plannord; `null` für ein waagerechtes Teil, unentschieden, wo es \
                     waagerecht sein oder senkrecht stehen kann.",
                ),
            ),
        ],
    },
    walking::FLIGHTS_ITEM,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "free_placements",
            parameters: &[
                MeasuredParameter {
                    key: "shape",
                    kind: MeasuredParameterKind::Choice {
                        options: &["circle", "rectangle"],
                    },
                    required: true,
                    default: None,
                    help: &en_de(
                        "The shape placed: a circle of `diameter`, or a rectangle `width` by \
                         `length` in any orientation, each `height` high.",
                        "Die platzierte Form: ein Kreis mit `diameter` oder ein Rechteck \
                         `width` mal `length` in beliebiger Ausrichtung, jeweils `height` hoch.",
                    ),
                },
                optional_length(
                    "diameter",
                    &en_de("The circle's diameter.", "Der Durchmesser des Kreises."),
                ),
                optional_length(
                    "width",
                    &en_de("The rectangle's width.", "Die Breite des Rechtecks."),
                ),
                optional_length(
                    "length",
                    &en_de("The rectangle's length.", "Die Länge des Rechtecks."),
                ),
                required_length(
                    "height",
                    &en_de("The shape's height.", "Die Höhe der Form."),
                ),
                optional_kinds(
                    "obstacles",
                    &en_de(
                        "The source kinds that obstruct the floor; every other object without it.",
                        "Die Quellarten, die den Boden verstellen; ohne Angabe jedes andere \
                         Objekt.",
                    ),
                ),
                optional_length(
                    "band_from",
                    &en_de(
                        "Where above the floor obstacles start to count.",
                        "Ab welcher Höhe über dem Boden Hindernisse zählen.",
                    ),
                ),
                optional_length(
                    "band_to",
                    &en_de(
                        "Where above the floor obstacles stop counting.",
                        "Bis zu welcher Höhe über dem Boden Hindernisse zählen.",
                    ),
                ),
                MeasuredParameter {
                    key: "merge",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The relationship steps to the spaces searched with it.",
                        "Die Beziehungsschritte zu den mitdurchsuchten Räumen.",
                    ),
                },
                optional_kinds(
                    "swings",
                    &en_de(
                        "The source kinds of doors whose swings obstruct the floor.",
                        "Die Quellarten der Türen, deren Aufschlag den Boden verstellt.",
                    ),
                ),
                optional_length(
                    "entrance_width",
                    &en_de(
                        "How wide a path from an entrance must reach the shape.",
                        "Wie breit ein Weg von einem Zugang die Form erreichen muss.",
                    ),
                ),
                MeasuredParameter {
                    key: "access",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the space to its entrances.",
                        "Die Beziehungsschritte vom Raum zu seinen Zugängen.",
                    ),
                },
                optional_kinds(
                    "doors",
                    &en_de(
                        "The source kinds of entrance doors.",
                        "Die Quellarten der Zugangstüren.",
                    ),
                ),
                optional_kinds(
                    "openings",
                    &en_de(
                        "The source kinds of entrance openings.",
                        "Die Quellarten der Zugangsöffnungen.",
                    ),
                ),
            ],
            dimension: None,
            services: &["free-space", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &["the free-space service cannot search the floor"],
            label: &en_de("Free placements", "Freie Stellflächen"),
            help: &en_de(
                "A placement of the shape on the space's free floor: one when one is found, \
                 none when none can be, and one undecided when the selections leave it open, \
                 so `count` of them at least 1 is whether the shape fits.",
                "Eine Platzierung der Form auf der freien Bodenfläche des Raums: eine, wenn \
                 eine gefunden wird, keine, wenn keine möglich ist, und eine unentschiedene, \
                 wenn die Auswahlen es offenlassen; `count` mindestens 1 ist also, ob die Form \
                 passt.",
            ),
        },
        fields: &[],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "guard_edges",
            parameters: &[
                guard_length(
                    "barrier_gap",
                    &en_de(
                        "The widest gap between barriers that still guards the edge.",
                        "Die größte Lücke zwischen Absturzsicherungen, die die Kante noch \
                         sichert.",
                    ),
                ),
                guard_length(
                    "platform_gap",
                    &en_de(
                        "How far from the edge a barrier may stand.",
                        "Wie weit von der Kante eine Absturzsicherung stehen darf.",
                    ),
                ),
                guard_length(
                    "landing_gap",
                    &en_de(
                        "How far from the edge a landing below may lie.",
                        "Wie weit von der Kante eine tiefere Auftrittsfläche liegen darf.",
                    ),
                ),
                guard_length(
                    "landing_width",
                    &en_de(
                        "How wide a landing below must be to stand on.",
                        "Wie breit eine tiefere Auftrittsfläche sein muss, um darauf zu \
                         stehen.",
                    ),
                ),
                guard_length(
                    "climb_distance",
                    &en_de(
                        "How close to a barrier an object may be climbed.",
                        "Wie nah an einer Absturzsicherung ein Objekt beklettert werden kann.",
                    ),
                ),
                guard_length(
                    "climb_side",
                    &en_de(
                        "How long an object's shortest side must be to stand on.",
                        "Wie lang die kürzeste Seite eines Objekts sein muss, um darauf zu \
                         stehen.",
                    ),
                ),
                MeasuredParameter {
                    key: "measure_from",
                    kind: MeasuredParameterKind::Choice {
                        options: &["floor", "curb"],
                    },
                    required: false,
                    default: Some("floor"),
                    help: &en_de(
                        "Whether a barrier on a curb is measured from the floor or from the \
                         curb.",
                        "Ob eine Absturzsicherung auf einer Aufkantung vom Boden oder von \
                         der Aufkantung gemessen wird.",
                    ),
                },
                guard_kinds(
                    "barriers",
                    &en_de(
                        "The source kinds that may be barriers; any nearby body without it.",
                        "Die Quellarten, die Absturzsicherungen sein können; ohne Angabe \
                         jeder nahe Körper.",
                    ),
                ),
                guard_kinds(
                    "landings",
                    &en_de(
                        "The source kinds that may be landings below; any without it.",
                        "Die Quellarten, die tiefere Auftrittsflächen sein können; ohne \
                         Angabe jede.",
                    ),
                ),
                guard_kinds(
                    "climbables",
                    &en_de(
                        "The source kinds that may be climbed; any without it.",
                        "Die Quellarten, die beklettert werden können; ohne Angabe jede.",
                    ),
                ),
            ],
            dimension: None,
            services: &["guard", "type-hierarchy"],
            exactness: MeasuredExactness::Stated,
            not_evaluated: &[
                "the guard service cannot measure the surface's exposed edges",
                "no edge of the surface is measured",
            ],
            label: &en_de("Exposed edges", "Absturzkanten"),
            help: &en_de(
                "The exposed edges of a walking surface, as the guard service samples them, \
                 each with what guards it: the height its barriers reach along all of it, \
                 the share they reach along at all, the fall onto landings covering it, and \
                 the lowest object beside a barrier to climb.",
                "Die Absturzkanten einer Lauffläche, wie der Absturzdienst sie abtastet, \
                 jede mit dem, was sie sichert: die Höhe, die ihre Absturzsicherungen \
                 entlang der ganzen Kante erreichen, der Anteil, den sie überhaupt \
                 erreichen, die Fallhöhe auf deckende Auftrittsflächen und das niedrigste \
                 bekletterbare Objekt neben einer Absturzsicherung.",
            ),
        },
        fields: &[
            field(
                "guarded_height",
                LENGTH,
                &en_de("Guarded height", "Gesicherte Höhe"),
                &en_de(
                    "The greatest height such that barriers at least that tall cover the whole \
                     edge; `null` when barriers never do.",
                    "Die größte Höhe, bei der mindestens so hohe Absturzsicherungen die ganze \
                     Kante decken; `null`, wenn sie es nie tun.",
                ),
            ),
            field(
                "tallest_barrier",
                LENGTH,
                &en_de("Tallest barrier", "Höchste Absturzsicherung"),
                &en_de(
                    "The height of the tallest barrier within `platform_gap`; `null` with none.",
                    "Die Höhe der höchsten Absturzsicherung innerhalb von `platform_gap`; \
                     `null` ohne.",
                ),
            ),
            field(
                "barrier_share",
                RATIO,
                &en_de("Barrier share", "Anteil mit Absturzsicherung"),
                &en_de(
                    "The share of the edge barriers within `platform_gap` run along, whatever \
                     their height.",
                    "Der Anteil der Kante, an dem Absturzsicherungen innerhalb von \
                     `platform_gap` verlaufen, gleich welcher Höhe.",
                ),
            ),
            field(
                "landing_fall",
                LENGTH,
                &en_de("Fall onto landings", "Fallhöhe auf Auftrittsflächen"),
                &en_de(
                    "The least fall such that landings no deeper, wide enough and close \
                     enough cover the whole edge; `null` when they never do.",
                    "Die kleinste Fallhöhe, bei der nicht tiefere, ausreichend breite und \
                     nahe Auftrittsflächen die ganze Kante decken; `null`, wenn sie es nie \
                     tun.",
                ),
            ),
            field(
                "climbable_height",
                LENGTH,
                &en_de(
                    "Lowest climbable object",
                    "Niedrigstes bekletterbares Objekt",
                ),
                &en_de(
                    "The height of the lowest object within `climb_distance` of a barrier \
                     with a side of at least `climb_side`; `null` with none.",
                    "Die Höhe des niedrigsten Objekts innerhalb von `climb_distance` einer \
                     Absturzsicherung mit einer Seite von mindestens `climb_side`; `null` \
                     ohne.",
                ),
            ),
        ],
    },
    walking::HANDRAIL_STRETCHES,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "handrails",
            parameters: &[
                MeasuredParameter {
                    key: "rails",
                    kind: MeasuredParameterKind::SourceKind,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The source kinds that may be handrails, `,`-separated, subtypes \
                         included.",
                        "Die Quellarten, die Handläufe sein können, durch `,` getrennt, \
                         Untertypen eingeschlossen.",
                    ),
                },
                required_length(
                    "reach_across",
                    &en_de(
                        "How far outside the walking surface's sides a rail may run.",
                        "Wie weit außerhalb der Seiten der Lauffläche ein Handlauf liegen darf.",
                    ),
                ),
                required_length(
                    "reach_above",
                    &en_de(
                        "How far above the pitch line a rail may run.",
                        "Wie weit über der Steigungslinie ein Handlauf liegen darf.",
                    ),
                ),
                MeasuredParameter {
                    key: "level_over",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: Some("0"),
                    help: &en_de(
                        "The extension over which a rail's rise beyond each end is measured.",
                        "Die Verlängerung, über die der Anstieg eines Handlaufs an jedem Ende \
                         gemessen wird.",
                    ),
                },
                MeasuredParameter {
                    key: "from",
                    kind: MeasuredParameterKind::Choice {
                        options: &["nosing", "riser"],
                    },
                    required: false,
                    default: Some("nosing"),
                    help: &en_de(
                        "Whether a flight's extensions are measured from its first and last \
                         nosing or riser.",
                        "Ob die Verlängerungen eines Laufs von der ersten und letzten \
                         Stufenvorderkante oder Steigung gemessen werden.",
                    ),
                },
                MeasuredParameter {
                    key: "of",
                    kind: MeasuredParameterKind::Choice {
                        options: &["flight", "ramp"],
                    },
                    required: false,
                    default: Some("flight"),
                    help: &en_de(
                        "Whether the object is a stair flight or a ramp, whose runs are \
                         taken one after another.",
                        "Ob das Objekt ein Treppenlauf oder eine Rampe ist, deren Läufe \
                         nacheinander genommen werden.",
                    ),
                },
            ],
            dimension: None,
            services: FLIGHTS,
            exactness: MeasuredExactness::Measured,
            not_evaluated: &[
                "the walking-surface service cannot measure the flight's or ramp's handrails",
                "the pieces along a side lie beside one another, not one after another",
            ],
            label: &en_de("Handrails", "Handläufe"),
            help: &en_de(
                "Each rail along a flight, or along each run of a ramp: its height above the \
                 pitch line, how far it reaches beyond each end, which side it runs along \
                 and the gap to the next piece of the handrail there.",
                "Jeder Handlauf entlang eines Laufs oder jedes Rampenlaufs: seine Höhe über \
                 der Steigungslinie, wie weit er über jedes Ende reicht, an welcher Seite er \
                 verläuft und die Lücke zum nächsten Stück des Handlaufs dort.",
            ),
        },
        fields: &[
            field(
                "run",
                RATIO,
                &en_de("Run", "Lauf"),
                &en_de(
                    "The run the rail runs along, counted from 1; 1 on a flight.",
                    "Der Lauf, an dem der Handlauf verläuft, ab 1 gezählt; 1 bei einem \
                     Treppenlauf.",
                ),
            ),
            field(
                "left",
                TRUTH,
                &en_de("Along the left side", "An der linken Seite"),
                &en_de(
                    "Whether it runs along the left side, seen climbing.",
                    "Ob er an der linken Seite verläuft, steigend gesehen.",
                ),
            ),
            field(
                "right",
                TRUTH,
                &en_de("Along the right side", "An der rechten Seite"),
                &en_de(
                    "Whether it runs along the right side, seen climbing.",
                    "Ob er an der rechten Seite verläuft, steigend gesehen.",
                ),
            ),
            field(
                "height_lowest",
                LENGTH,
                &en_de("Lowest height", "Geringste Höhe"),
                &en_de(
                    "Its top's least height above the pitch line.",
                    "Die geringste Höhe seiner Oberkante über der Steigungslinie.",
                ),
            ),
            field(
                "height_highest",
                LENGTH,
                &en_de("Highest height", "Größte Höhe"),
                &en_de(
                    "Its top's greatest height above the pitch line.",
                    "Die größte Höhe seiner Oberkante über der Steigungslinie.",
                ),
            ),
            field(
                "extension_bottom",
                LENGTH,
                &en_de("Extension at the bottom", "Verlängerung unten"),
                &en_de(
                    "How far it reaches level beyond the bottom end; `null` when it runs \
                     along a later part of a turning flight only.",
                    "Wie weit er waagrecht über das untere Ende reicht; `null`, wenn er nur \
                     an einem späteren Teil eines gewendelten Laufs verläuft.",
                ),
            ),
            field(
                "extension_top",
                LENGTH,
                &en_de("Extension at the top", "Verlängerung oben"),
                &en_de(
                    "How far it reaches level beyond the top end; `null` when it runs along \
                     an earlier part of a turning flight only.",
                    "Wie weit er waagrecht über das obere Ende reicht; `null`, wenn er nur \
                     an einem früheren Teil eines gewendelten Laufs verläuft.",
                ),
            ),
            field(
                "bottom_rise",
                LENGTH,
                &en_de("Rise beyond the bottom", "Anstieg unten"),
                &en_de(
                    "How far its top rises or falls over `level_over` beyond the bottom end.",
                    "Wie weit seine Oberkante über `level_over` jenseits des unteren Endes \
                     steigt oder fällt.",
                ),
            ),
            field(
                "top_rise",
                LENGTH,
                &en_de("Rise beyond the top", "Anstieg oben"),
                &en_de(
                    "How far its top rises or falls over `level_over` beyond the top end.",
                    "Wie weit seine Oberkante über `level_over` jenseits des oberen Endes \
                     steigt oder fällt.",
                ),
            ),
            field(
                "first_on_side",
                TRUTH,
                &en_de("First piece", "Erstes Stück"),
                &en_de(
                    "Whether it is the lowest piece of the handrail along its side; false \
                     over the middle.",
                    "Ob er das unterste Stück des Handlaufs an seiner Seite ist; falsch über \
                     der Mitte.",
                ),
            ),
            field(
                "last_on_side",
                TRUTH,
                &en_de("Last piece", "Letztes Stück"),
                &en_de(
                    "Whether it is the highest piece of the handrail along its side; false \
                     over the middle.",
                    "Ob er das oberste Stück des Handlaufs an seiner Seite ist; falsch über \
                     der Mitte.",
                ),
            ),
            field(
                "gap_after",
                LENGTH,
                &en_de("Gap to the next piece", "Lücke zum nächsten Stück"),
                &en_de(
                    "The plan gap to the next piece along its side; `null` for the last \
                     piece and over the middle.",
                    "Die Lücke im Grundriss zum nächsten Stück an seiner Seite; `null` für \
                     das letzte Stück und über der Mitte.",
                ),
            ),
        ],
    },
    walking::LANDING_DOORS,
    walking::LANDING_SWINGS,
    walking::LANDINGS,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "opening_placements",
            parameters: &[
                MeasuredParameter {
                    key: "host_path",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the opening to its hosts.",
                        "Die Beziehungsschritte von der Öffnung zu ihren Wirten.",
                    ),
                },
                MeasuredParameter {
                    key: "hosts",
                    kind: MeasuredParameterKind::SourceKind,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The source kinds of the hosts, `,`-separated, subtypes included; any \
                         object the path reaches without it.",
                        "Die Quellarten der Wirte, durch `,` getrennt, Untertypen \
                         eingeschlossen; ohne Angabe jedes erreichte Objekt.",
                    ),
                },
                FACE_AXES[0],
                FACE_AXES[1],
                MeasuredParameter {
                    key: "zone",
                    kind: MeasuredParameterKind::Choice {
                        options: &["section", "web"],
                    },
                    required: false,
                    default: Some("section"),
                    help: &en_de(
                        "Whether edge distances are measured to the host's edges or to its \
                         flanges (`web`, across `profile-y`).",
                        "Ob Randabstände zu den Rändern des Wirts oder zu seinen Flanschen \
                         (`web`, quer zu `profile-y`) gemessen werden.",
                    ),
                },
                OPENINGS_MINIMUM,
            ],
            dimension: None,
            services: &["relationship-selection", "body-facts"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &[
                "whether a reached object is a host is undecided",
                "the opening or its host is no straight extrusion the body set bounds",
            ],
            label: &en_de("Opening placements", "Lagen einer Öffnung"),
            help: &en_de(
                "The opening's placement in each host its path reaches, as `opening-zone` \
                 places it: whether it lies within the host's face, and its clear distances \
                 from the host's ends and edges. A distance measured to a free outline from \
                 the box the opening may lie in is a lower bound, widened up to the host's \
                 extent; a placement that cannot be read, or may be below the minimum area, \
                 states every field undecided.",
                "Die Lage der Öffnung in jedem Wirt, den ihr Pfad erreicht, wie \
                 `opening-zone` sie bestimmt: ob sie in der Ansicht des Wirts liegt, und ihre \
                 lichten Abstände zu seinen Enden und Rändern. Ein Abstand zu einem freien \
                 Umriss aus dem Quader, in dem die Öffnung liegen kann, ist eine Untergrenze, \
                 bis zur Ausdehnung des Wirts erweitert; eine nicht lesbare oder womöglich zu \
                 kleine Lage gibt jedes Feld unentschieden an.",
            ),
        },
        fields: &[
            field(
                "inside",
                TRUTH,
                &en_de("Inside", "Innerhalb"),
                &en_de(
                    "Whether the opening lies within the host's face (and free outline).",
                    "Ob die Öffnung innerhalb der Ansicht (und des freien Umrisses) des \
                     Wirts liegt.",
                ),
            ),
            field(
                "end_distance",
                LENGTH,
                &en_de("End distance", "Endabstand"),
                &en_de(
                    "The clear distance from the nearer end of the host along its length; \
                     `null` where the outline bounds no end across the opening.",
                    "Der lichte Abstand zum näheren Ende des Wirts entlang seiner Länge; \
                     `null`, wo der Umriss kein Ende quer zur Öffnung begrenzt.",
                ),
            ),
            field(
                "edge_distance",
                LENGTH,
                &en_de("Edge distance", "Randabstand"),
                &en_de(
                    "The clear distance from the nearer edge (or flange, with `zone` `web`), \
                     negative where the opening reaches into a flange.",
                    "Der lichte Abstand zum näheren Rand (oder Flansch, mit `zone` `web`), \
                     negativ, wo die Öffnung in einen Flansch reicht.",
                ),
            ),
            field(
                "bottom_distance",
                LENGTH,
                &en_de("Bottom distance", "Abstand unten"),
                &en_de(
                    "The distance from the host's low edge (or lower flange) along its \
                     height.",
                    "Der Abstand zum unteren Rand (oder unteren Flansch) des Wirts entlang \
                     seiner Höhe.",
                ),
            ),
            field(
                "top_distance",
                LENGTH,
                &en_de("Top distance", "Abstand oben"),
                &en_de(
                    "The distance from the host's high edge (or upper flange) along its \
                     height.",
                    "Der Abstand zum oberen Rand (oder oberen Flansch) des Wirts entlang \
                     seiner Höhe.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "parallel_pairs",
            parameters: &[
                SPACED_MEMBERS,
                MEMBER_PATH,
                ANGLE_TOLERANCE,
                required_length(
                    "reach",
                    &en_de(
                        "How far apart in plan a pair is still listed, in metres: at \
                         least the widest spacing judged.",
                        "Bis zu welchem Abstand im Grundriss ein Paar aufgeführt wird, in \
                         Metern: mindestens der größte geprüfte Abstand.",
                    ),
                ),
            ],
            dimension: None,
            services: PAIRED,
            exactness: MeasuredExactness::Measured,
            not_evaluated: &[
                "a member's extent cannot be read",
                "whether two members are parallel and face each other is unknown",
            ],
            label: &en_de("Parallel pairs", "Parallele Paare"),
            help: &en_de(
                "The pairs of members the object reaches that are parallel and face each \
                 other along the first one's long axis, as `wall-spacing` pairs them; a \
                 pair only possibly parallel or facing is an undecided member.",
                "Die Paare erreichter Bauteile, die parallel sind und sich entlang der \
                 Längsachse des ersten gegenüberstehen, wie `wall-spacing` sie bildet; \
                 ein nur möglicherweise paralleles Paar ist ein unentschiedenes Element.",
            ),
        },
        fields: &[field(
            "distance",
            LENGTH,
            &en_de("Distance", "Abstand"),
            &en_de(
                "The pair's plan distance, closest points to closest points.",
                "Der Abstand des Paars im Grundriss, nächste Punkte zueinander.",
            ),
        )],
    },
    walking::RAIL_CONTINUITY,
    walking::RAIL_EXTENSIONS,
    walking::RAIL_GAPS,
    walking::RAIL_HEIGHTS,
    walking::RAIL_OBSTRUCTIONS,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "recesses",
            parameters: &[],
            dimension: None,
            services: &["plan-span"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &["the plan-span service does not measure the recesses"],
            label: &en_de("Recesses", "Rücksprünge"),
            help: &en_de(
                "The pockets between a footprint's outer boundary and its convex hull.",
                "Die Taschen zwischen dem Außenrand eines Grundrisses und seiner konvexen \
                 Hülle.",
            ),
        },
        fields: &[
            field(
                "width",
                LENGTH,
                &en_de("Mouth width", "Öffnungsbreite"),
                &en_de(
                    "The width of the recess's mouth.",
                    "Die Breite der Öffnung des Rücksprungs.",
                ),
            ),
            field(
                "depth",
                LENGTH,
                &en_de("Depth", "Tiefe"),
                &en_de(
                    "How deep the recess reaches behind its mouth.",
                    "Wie tief der Rücksprung hinter seine Öffnung reicht.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "runs",
            parameters: &[MeasuredParameter {
                key: "landing",
                kind: MeasuredParameterKind::SourceKind,
                required: false,
                default: None,
                help: &en_de(
                    "The source kinds that may carry a landing at a run's ends; without it \
                     the landing fields are undecided.",
                    "Die Quellarten, die ein Podest an den Enden eines Laufs tragen können; \
                     ohne Angabe sind die Podestfelder unentschieden.",
                ),
            }],
            dimension: None,
            services: FLIGHTS,
            exactness: MeasuredExactness::Measured,
            not_evaluated: &["the walking-surface service cannot measure the ramp's runs"],
            label: &en_de("Ramp runs", "Rampenläufe"),
            help: &en_de(
                "A ramp's sloped runs, lowest first.",
                "Die geneigten Läufe einer Rampe, der niedrigste zuerst.",
            ),
        },
        fields: &[
            MemberField {
                name: "run",
                kind: RATIO,
                label: &en_de("Run", "Lauf"),
                help: &en_de(
                    "The run's place, counted from 1 at the lowest.",
                    "Die Stelle des Laufs, ab 1 beim untersten gezählt.",
                ),
            },
            MemberField {
                name: "slope",
                kind: RATIO,
                label: &en_de("Slope", "Neigung"),
                help: &en_de(
                    "The run's rise over its horizontal length.",
                    "Die Steigung des Laufs über seine waagrechte Länge.",
                ),
            },
            MemberField {
                name: "length",
                kind: LENGTH,
                label: &en_de("Length", "Länge"),
                help: &en_de(
                    "The run's horizontal length along its direction.",
                    "Die waagrechte Länge des Laufs in seiner Richtung.",
                ),
            },
            MemberField {
                name: "rise",
                kind: LENGTH,
                label: &en_de("Rise", "Höhe"),
                help: &en_de("How far the run rises.", "Wie hoch der Lauf steigt."),
            },
            MemberField {
                name: "width",
                kind: LENGTH,
                label: &en_de("Width", "Breite"),
                help: &en_de(
                    "The run's width across its direction; `null` where its sides are \
                     not measured.",
                    "Die Breite des Laufs quer zu seiner Richtung; `null`, wo seine Seiten \
                     nicht gemessen sind.",
                ),
            },
            MemberField {
                name: "label",
                kind: MemberFieldKind::Text,
                label: &en_de("Label", "Bezeichnung"),
                help: &en_de(
                    "How messages name the run: `run 2 of 3`.",
                    "Wie Meldungen den Lauf nennen: `run 2 of 3`.",
                ),
            },
            MemberField {
                name: "scale",
                kind: RATIO,
                label: &en_de("Scale", "Größenordnung"),
                help: &en_de(
                    "The largest magnitude among the run's positions, in metres: the binary \
                     rounding of decimal coordinates grows with it.",
                    "Der größte Betrag unter den Lagen des Laufs, in Metern: die binäre \
                     Rundung dezimaler Koordinaten wächst mit ihm.",
                ),
            },
            MemberField {
                name: "slope_rounding",
                kind: RATIO,
                label: &en_de("Slope rounding", "Rundung der Neigung"),
                help: &en_de(
                    "How far the binary rounding of the run's coordinates may move its slope.",
                    "Wie weit die binäre Rundung der Koordinaten des Laufs seine Neigung \
                     verschieben kann.",
                ),
            },
            MemberField {
                name: "bottom_landing",
                kind: MemberFieldKind::Truth,
                label: &en_de("Landing at the bottom", "Podest unten"),
                help: &en_de(
                    "Whether a landing of the `landing` kinds meets the run's bottom.",
                    "Ob ein Podest der `landing`-Arten den Lauf unten trifft.",
                ),
            },
            MemberField {
                name: "bottom_landing_depth",
                kind: LENGTH,
                label: &en_de("Landing depth at the bottom", "Podesttiefe unten"),
                help: &en_de(
                    "How deep that landing is along the walking direction; `null` without one.",
                    "Wie tief dieses Podest in Gehrichtung ist; `null` ohne.",
                ),
            },
            MemberField {
                name: "bottom_landing_width",
                kind: LENGTH,
                label: &en_de("Landing width at the bottom", "Podestbreite unten"),
                help: &en_de(
                    "How wide that landing is across the walking direction; `null` without one.",
                    "Wie breit dieses Podest quer zur Gehrichtung ist; `null` ohne.",
                ),
            },
            MemberField {
                name: "top_landing",
                kind: MemberFieldKind::Truth,
                label: &en_de("Landing at the top", "Podest oben"),
                help: &en_de(
                    "Whether a landing of the `landing` kinds meets the run's top.",
                    "Ob ein Podest der `landing`-Arten den Lauf oben trifft.",
                ),
            },
            MemberField {
                name: "top_landing_depth",
                kind: LENGTH,
                label: &en_de("Landing depth at the top", "Podesttiefe oben"),
                help: &en_de(
                    "How deep that landing is along the walking direction; `null` without one.",
                    "Wie tief dieses Podest in Gehrichtung ist; `null` ohne.",
                ),
            },
            MemberField {
                name: "top_landing_width",
                kind: LENGTH,
                label: &en_de("Landing width at the top", "Podestbreite oben"),
                help: &en_de(
                    "How wide that landing is across the walking direction; `null` without one.",
                    "Wie breit dieses Podest quer zur Gehrichtung ist; `null` ohne.",
                ),
            },
        ],
    },
    walking::STAIR_CLEAR_WIDTHS,
    walking::STAIR_CONTINUITY,
    walking::STAIRS,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "steps",
            parameters: &[WALKING_LINE_OFFSET],
            dimension: None,
            services: FLIGHTS,
            exactness: MeasuredExactness::Measured,
            not_evaluated: &["the walking-surface service cannot measure the flight's treads"],
            label: &en_de("Flight steps", "Stufen eines Laufs"),
            help: &en_de(
                "A flight's steps, bottom to top: one per riser, from the flight's base \
                 to its first tread, between consecutive treads, and from its last tread \
                 to its top when it ends in a riser.",
                "Die Stufen eines Laufs von unten nach oben: eine je Steigung, vom Fuß \
                 des Laufs zur ersten Trittstufe, zwischen aufeinanderfolgenden \
                 Trittstufen und von der letzten zum Kopf, wenn er in einer Steigung \
                 endet.",
            ),
        },
        fields: &[
            MemberField {
                name: "riser",
                kind: LENGTH,
                label: &en_de("Riser", "Steigung"),
                help: &en_de("The step's riser height.", "Die Steigungshöhe der Stufe."),
            },
            MemberField {
                name: "going",
                kind: LENGTH,
                label: &en_de("Going", "Auftritt"),
                help: &en_de(
                    "The step's going along the walking line, from the nosing below to \
                     its own; `null` for the first step and a final riser.",
                    "Der Auftritt der Stufe entlang der Lauflinie, von der Vorderkante \
                     darunter zu ihrer eigenen; `null` für die erste Stufe und eine \
                     Austrittsstufe.",
                ),
            },
            MemberField {
                name: "step_length",
                kind: LENGTH,
                label: &en_de("Step length (2r + g)", "Schrittmaß (2s + a)"),
                help: &en_de(
                    "Twice the riser plus the going; `null` where the going is.",
                    "Zweimal die Steigung plus der Auftritt; `null`, wo der Auftritt es \
                     ist.",
                ),
            },
            MemberField {
                name: "nosing",
                kind: LENGTH,
                label: &en_de("Nosing projection", "Unterschneidung"),
                help: &en_de(
                    "How far the step's tread reaches over the tread below, zero or less \
                     where it does not; `null` where the going is.",
                    "Wie weit die Trittstufe über die darunter reicht, null oder weniger, \
                     wo sie es nicht tut; `null`, wo der Auftritt es ist.",
                ),
            },
            MemberField {
                name: "winder_angle",
                kind: ANGLE,
                label: &en_de("Winder angle", "Wendelwinkel"),
                help: &en_de(
                    "The plan angle between the nosing below and the step's own, zero for \
                     a straight tread; `null` where the going is or a nosing is not \
                     measured.",
                    "Der Grundrisswinkel zwischen der Vorderkante darunter und der eigenen, \
                     null für eine gerade Stufe; `null`, wo der Auftritt es ist oder eine \
                     Vorderkante nicht gemessen ist.",
                ),
            },
            MemberField {
                name: "turning",
                kind: MemberFieldKind::Truth,
                label: &en_de("Turning", "Gewendelt"),
                help: &en_de(
                    "Whether the flight's walking line turns.",
                    "Ob die Lauflinie des Laufs sich wendet.",
                ),
            },
            MemberField {
                name: "open_riser",
                kind: MemberFieldKind::Truth,
                label: &en_de("Open riser", "Offene Steigung"),
                help: &en_de(
                    "Whether the riser leaves the step open.",
                    "Ob die Steigung die Stufe offen lässt.",
                ),
            },
            MemberField {
                name: "scale",
                kind: RATIO,
                label: &en_de("Scale", "Größenordnung"),
                help: &en_de(
                    "The largest magnitude among the flight's positions, in metres: the \
                     binary rounding of decimal coordinates grows with it.",
                    "Der größte Betrag unter den Lagen des Laufs, in Metern: die binäre \
                     Rundung dezimaler Koordinaten wächst mit ihm.",
                ),
            },
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "swing_spaces",
            parameters: &[
                MeasuredParameter {
                    key: "path",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the door to the spaces it opens onto.",
                        "Die Beziehungsschritte von der Tür zu den Räumen, zu denen sie sich \
                         öffnet.",
                    ),
                },
                MeasuredParameter {
                    key: "kinds",
                    kind: MeasuredParameterKind::SourceKind,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The source kinds of the spaces listed, `,`-separated, subtypes \
                         included; every space reached without it.",
                        "Die Quellarten der aufgeführten Räume, durch `,` getrennt, Untertypen \
                         eingeschlossen; ohne Angabe jeder erreichte Raum.",
                    ),
                },
            ],
            dimension: None,
            services: &["object-frame", "free-space", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &[
                "the door's leaves are unknown",
                "the door has no hinged leaf",
                "a probe the free-space service cannot answer",
            ],
            label: &en_de("Spaces a door opens onto", "Räume an einer Tür"),
            help: &en_de(
                "The spaces the path reaches from a door, each with whether the door swings \
                 into it and whether it swings away from it, probed as `door-swing` probes \
                 them.",
                "Die Räume, die der Pfad von einer Tür erreicht, je damit, ob die Tür in ihn \
                 hinein oder von ihm weg aufschlägt, geprüft wie `door-swing` prüft.",
            ),
        },
        fields: &[
            field(
                "into",
                TRUTH,
                &en_de("Swings into", "Schlägt hinein"),
                &en_de(
                    "Whether a leaf swings into the space (a double-acting leaf into both \
                     sides); false where neither probe lies in it.",
                    "Ob ein Flügel in den Raum aufschlägt (ein Pendelflügel zu beiden \
                     Seiten); falsch, wo keine Probe in ihm liegt.",
                ),
            ),
            field(
                "away",
                TRUTH,
                &en_de("Swings away", "Schlägt weg"),
                &en_de(
                    "Whether the space surely lies behind the door and not on its swing side; \
                     undecided where neither probe lies in it.",
                    "Ob der Raum sicher hinter der Tür und nicht auf ihrer Aufschlagseite \
                     liegt; unentschieden, wo keine Probe in ihm liegt.",
                ),
            ),
        ],
    },
    walking::TACTILE_STRIPS,
];

/// The member list `name` (without parameters), ignoring ASCII case.
#[must_use]
pub fn member_descriptor(name: &str) -> Option<&'static MemberDescriptor> {
    MEASURED_MEMBERS
        .iter()
        .find(|descriptor| descriptor.list.name.eq_ignore_ascii_case(name.trim()))
}

/// Parses `name[;key=value…]` against [`MEASURED_MEMBERS`]; the call's
/// descriptor is the list's.
///
/// # Errors
///
/// As [`super::parse`]: an unknown list, or a parameter it does not take
/// or of the wrong kind.
pub fn parse_members(text: &str) -> Result<MeasuredCall, MeasuredError> {
    // A member list names the rule's parameters (`@name`) and the anchor
    // as a measured value does; they are bound when the rule reads it.
    super::parse_in(
        text,
        MEASURED_MEMBERS.iter().map(|descriptor| &descriptor.list),
        "measured member list",
    )
}

/// The member list `call` was parsed from.
#[must_use]
pub fn members_of(call: &MeasuredCall) -> Option<&'static MemberDescriptor> {
    member_descriptor(call.name())
}

/// Whether some member list states the field `name` of [`MEMBER_SET`](crate::MEMBER_SET).
#[must_use]
pub fn is_member_field(name: &str) -> bool {
    MEASURED_MEMBERS
        .iter()
        .any(|descriptor| descriptor.field(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_sorted_labelled_and_their_fields_unique() {
        let names: Vec<_> = MEASURED_MEMBERS.iter().map(|d| d.list.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
        for descriptor in MEASURED_MEMBERS {
            assert_eq!(descriptor.list.dimension, None);
            let mut fields: Vec<_> = descriptor.fields.iter().map(|f| f.name).collect();
            fields.sort_unstable();
            fields.dedup();
            assert_eq!(fields.len(), descriptor.fields.len());
            for texts in descriptor
                .fields
                .iter()
                .flat_map(|field| [field.label, field.help])
                .chain([descriptor.list.label, descriptor.list.help])
            {
                let languages: Vec<_> = texts.iter().map(|t| t.language).collect();
                assert_eq!(languages, ["en", "de"], "{}", descriptor.list.name);
            }
            // A member list is no measured value.
            assert!(super::super::descriptor(descriptor.list.name).is_none());
        }
        assert_eq!(crate::MEMBER_SET, "axioval:member");
    }

    #[test]
    fn lists_parse_like_measured_values() {
        let call = parse_members("Steps;walking_line_offset=0.3").unwrap();
        assert_eq!(call.name(), "steps");
        assert!(members_of(&call).unwrap().field("riser").is_some());
        let error = parse_members("treads").unwrap_err().to_string();
        assert!(
            error.starts_with("`treads` is no measured member list; known: "),
            "{error}"
        );
        assert!(is_member_field("step_length"));
        assert!(!is_member_field("treads"));
    }
}
