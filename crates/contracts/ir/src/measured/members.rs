//! Measured member lists: the parts of an object built-in code measures
//! one by one (a flight's steps, a ramp's runs), which an aggregate ranges
//! over and whose fields its expressions read in [`MEMBER_SET`](crate::MEMBER_SET).
//!
//! A list is written like a measured value, `name[;key=value…]`, and
//! parsed against [`MEASURED_MEMBERS`] with [`parse_members`].

use serde::Serialize;

use super::registry::{
    ANGLE_TOLERANCE, COORDINATE_DIFFERENCES, COORDINATES, COORDINATES_UNREAD, EFFECT_SERVICES,
    EFFECT_UNMEASURED, EFFECTIVE, FACE, FACE_AXES, FACING, MEMBER_PATH, NO_FACE, NO_GEOMETRY,
    OPENINGS_MINIMUM, PAIRED, SPACE, SPACE_SELECTED, SPACE_TOLERANCE, SPACE_UNDECIDED,
    SPACE_UNMEASURED, SPACED_MEMBERS, TRAVERSAL, WELL_MEMBERS, en_de,
};
use super::{
    FACE_PIECES, LocalizedText, MeasuredCall, MeasuredDescriptor, MeasuredError, MeasuredExactness,
    MeasuredParameter, MeasuredParameterKind, MeasuredSubject,
};
use crate::QuantityDimension;

mod clash;
mod searches;
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
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help,
    }
}

/// The search the guard service measures exposed edges under, shared by
/// `guard_edges` and `guard_surfaces`.
pub(super) const GUARD: [MeasuredParameter; 13] = [
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
            "The objects that may be barriers: their source kinds, or `@` a \
                         selector parameter of the rule, whose undecided objects leave the \
                         search refused; any nearby body without it.",
            "Die Objekte, die Absturzsicherungen sein können: ihre Quellarten oder \
                         mit `@` ein Selektorparameter der Regel, dessen unentschiedene Objekte \
                         die Suche ablehnen lassen; ohne Angabe jeder nahe Körper.",
        ),
    ),
    guard_kinds(
        "landings",
        &en_de(
            "The objects that may be landings below, as `barriers` names them; \
                         any without it.",
            "Die Objekte, die tiefere Auftrittsflächen sein können, benannt wie \
                         bei `barriers`; ohne Angabe jede.",
        ),
    ),
    guard_kinds(
        "climbables",
        &en_de(
            "The objects that may be climbed, as `barriers` names them; any \
                         without it.",
            "Die Objekte, die beklettert werden können, benannt wie bei \
                         `barriers`; ohne Angabe jede.",
        ),
    ),
    guard_kinds(
        "surfaces",
        &en_de(
            "The walking surfaces measured together in one request: their source kinds, \
             `,`-separated, or `@` a selector parameter of the rule (`@selection`, in a \
             template, the rule's own selection); without it, the object alone. With it, \
             role kinds leave no surface out.",
            "Die in einer Anfrage gemeinsam gemessenen Laufflächen: ihre Quellarten, durch \
             `,` getrennt, oder mit `@` ein Selektorparameter der Regel (`@selection`, in \
             einer Vorlage, die eigene Auswahl der Regel); ohne Angabe das Objekt allein. \
             Mit Angabe lassen Rollenarten keine Lauffläche aus.",
        ),
    ),
    MeasuredParameter {
        key: "from_curb",
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help: &en_de(
            "Whether a barrier on a curb is measured from the curb (`true`), as \
             `measure_from` `curb` states it: a rule's boolean.",
            "Ob eine Absturzsicherung auf einer Aufkantung von der Aufkantung gemessen wird \
             (`true`), wie `measure_from` `curb` es angibt: ein Wahrheitswert der Regel.",
        ),
    },
    MeasuredParameter {
        key: "climb_height",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "How tall an object beside a barrier may be and still be climbed: names the first \
             such object (`climbable`); none without it.",
            "Wie hoch ein Objekt neben einer Absturzsicherung sein darf, um noch beklettert \
             zu werden: benennt das erste solche Objekt (`climbable`); ohne Angabe keines.",
        ),
    },
];

/// The services `opening-zone`'s lists read.
const ZONE_SERVICES: &[&str] = &["relationship-selection", "body-facts", "proximity"];

/// The arguments of `opening-zone`'s lists: the rule's declaration by the
/// parameters' own names, and the openings it selects.
const ZONE: [MeasuredParameter; 21] = [
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
        key: "host_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The objects that are hosts; any object the path reaches without it.",
            "Die Objekte, die Wirte sind; ohne Angabe jedes erreichte Objekt.",
        ),
    },
    MeasuredParameter {
        key: "length_axis",
        kind: MeasuredParameterKind::Choice {
            options: &["extrusion", "profile-x", "profile-y"],
        },
        required: false,
        default: None,
        help: &en_de(
            "The host's axis along its length.",
            "Die Achse des Wirts entlang seiner Länge.",
        ),
    },
    MeasuredParameter {
        key: "height_axis",
        kind: MeasuredParameterKind::Choice {
            options: &["extrusion", "profile-x", "profile-y"],
        },
        required: false,
        default: None,
        help: &en_de(
            "The host's axis along its height.",
            "Die Achse des Wirts entlang seiner Höhe.",
        ),
    },
    MeasuredParameter {
        key: "end_distance",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "The least clear distance from the host's ends, in metres.",
            "Der kleinste lichte Abstand zu den Enden des Wirts, in Metern.",
        ),
    },
    MeasuredParameter {
        key: "edge_distance",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "The least clear distance from the host's edges (or flanges), in metres.",
            "Der kleinste lichte Abstand zu den Rändern (oder Flanschen) des Wirts, in Metern.",
        ),
    },
    MeasuredParameter {
        key: "edge_distance_maximum",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "The greatest distance from the host's bottom and top edges, in metres.",
            "Der größte Abstand zum unteren und oberen Rand des Wirts, in Metern.",
        ),
    },
    MeasuredParameter {
        key: "maximum_edges",
        kind: MeasuredParameterKind::Choice {
            options: &["top", "bottom", "both"],
        },
        required: false,
        default: None,
        help: &en_de(
            "Which edges `edge_distance_maximum` bounds the distance from.",
            "Von welchen Rändern `edge_distance_maximum` den Abstand begrenzt.",
        ),
    },
    MeasuredParameter {
        key: "zone",
        kind: MeasuredParameterKind::Choice {
            options: &["section", "web"],
        },
        required: false,
        default: None,
        help: &en_de(
            "Whether edge distances are measured to the host's edges or to its flanges (`web`, across `profile-y`).",
            "Ob Randabstände zu den Rändern des Wirts oder zu seinen Flanschen (`web`, quer zu `profile-y`) gemessen werden.",
        ),
    },
    MeasuredParameter {
        key: "opening_spacing",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "The least clear distance to another opening of the host, in metres.",
            "Der kleinste lichte Abstand zu einer anderen Öffnung des Wirts, in Metern.",
        ),
    },
    MeasuredParameter {
        key: "zones",
        kind: MeasuredParameterKind::Table,
        required: false,
        default: None,
        help: &en_de(
            "The rule's allowed zones: per row the insets from the ends, the bottom and the top.",
            "Die erlaubten Zonen der Regel: je Zeile die Abstände von den Enden, von unten und von oben.",
        ),
    },
    MeasuredParameter {
        key: "minimum_opening_area",
        kind: MeasuredParameterKind::Area { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "Openings smaller than this many square metres are not judged.",
            "Öffnungen kleiner als so viele Quadratmeter werden nicht geprüft.",
        ),
    },
    MeasuredParameter {
        key: "dimensions",
        kind: MeasuredParameterKind::Table,
        required: false,
        default: None,
        help: &en_de(
            "The rule's dimensioning rows: the distance from an opening to another opening or to an edge of the host, and its bounds.",
            "Die Bemaßungszeilen der Regel: der Abstand einer Öffnung zu einer anderen Öffnung oder zu einem Rand des Wirts, und seine Grenzen.",
        ),
    },
    MeasuredParameter {
        key: "support_path",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "The relationship steps from the host to its supports.",
            "Die Beziehungsschritte vom Wirt zu seinen Auflagern.",
        ),
    },
    MeasuredParameter {
        key: "support_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The objects that may be supports; any object without it.",
            "Die Objekte, die Auflager sein können; ohne Angabe jedes Objekt.",
        ),
    },
    MeasuredParameter {
        key: "support_gap",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "How near the host an object must come in space to be a support, in metres.",
            "Wie nah ein Objekt dem Wirt im Raum kommen muss, um ein Auflager zu sein, in Metern.",
        ),
    },
    MeasuredParameter {
        key: "support_distance",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "The least distance along the host from each support, in metres.",
            "Der kleinste Abstand entlang des Wirts zu jedem Auflager, in Metern.",
        ),
    },
    MeasuredParameter {
        key: "support_distance_ratio",
        kind: MeasuredParameterKind::Number { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "The least distance from each support as a share of the host's span or depth.",
            "Der kleinste Abstand zu jedem Auflager als Anteil der Spannweite oder Höhe des Wirts.",
        ),
    },
    MeasuredParameter {
        key: "support_distance_reference",
        kind: MeasuredParameterKind::Choice {
            options: &["span", "depth"],
        },
        required: false,
        default: None,
        help: &en_de(
            "What `support_distance_ratio` is a share of.",
            "Wovon `support_distance_ratio` ein Anteil ist.",
        ),
    },
    MeasuredParameter {
        key: "support_clearance",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "The least clearance from each support's footprint in the host's face, in metres.",
            "Der kleinste lichte Abstand zur Ansichtsfläche jedes Auflagers in der Ansicht des Wirts, in Metern.",
        ),
    },
    MeasuredParameter {
        key: "openings",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The openings the rule judges, each the others' neighbour; any object without it.",
            "Die Öffnungen, die die Regel prüft, jede Nachbar der anderen; ohne Angabe jedes Objekt.",
        ),
    },
];

/// Every list of measured members, sorted by name.
pub static MEASURED_MEMBERS: &[MemberDescriptor] = &[
    searches::ALLOCATIONS,
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
            subject: MeasuredSubject::Object,
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
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "centre_line_sides",
            parameters: &[
                MeasuredParameter {
                    key: "walls",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The walls beside the footprint: source kinds, `,`-separated, or the \
                         objects a selector parameter of the rule picks (`@name`), those it \
                         leaves undecided walls that may be there.",
                        "Die Wände neben dem Grundriss: Quellarten, durch `,` getrennt, oder \
                         die Objekte, die ein Selektorparameter der Regel wählt (`@name`), \
                         die unentschiedenen Wände, die dort sein können.",
                    ),
                },
                MeasuredParameter {
                    key: "centre_line",
                    kind: MeasuredParameterKind::Choice {
                        options: &["long", "short", "against-wall"],
                    },
                    required: true,
                    default: None,
                    help: &en_de(
                        "The footprint's centre line: along its long or short axis, or from \
                         the wall it stands against to its front.",
                        "Die Mittellinie des Grundrisses: entlang seiner langen oder kurzen \
                         Achse oder von der Wand, vor der es steht, zu seiner Vorderseite.",
                    ),
                },
                MeasuredParameter {
                    key: "sides",
                    kind: MeasuredParameterKind::Choice {
                        options: &["nearest", "both"],
                    },
                    required: true,
                    default: None,
                    help: &en_de(
                        "Both sides of the centre line, each an item, or the nearer of the \
                         two, one item.",
                        "Beide Seiten der Mittellinie, jede ein Element, oder die nähere der \
                         beiden, ein Element.",
                    ),
                },
                MeasuredParameter {
                    key: "reach",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: true,
                    default: None,
                    help: &en_de(
                        "How far from the centre line a wall is searched.",
                        "Wie weit von der Mittellinie eine Wand gesucht wird.",
                    ),
                },
                MeasuredParameter {
                    key: "inset",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far the strip beside the footprint is narrowed at both ends; not at \
                         all without it.",
                        "Um wie viel der Streifen neben dem Grundriss an beiden Enden \
                         verkürzt wird; ohne Angabe gar nicht.",
                    ),
                },
            ],
            dimension: None,
            services: &["plan-span", "type-hierarchy"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "the walls beside the footprint cannot be measured",
                "the footprint has no long axis",
                "no side is surely nearer a wall than the others",
            ],
            label: &en_de("Sides of the centre line", "Seiten der Mittellinie"),
            help: &en_de(
                "Each side of the centre line of the footprint's least-area rectangle a \
                 `centre-line-distance` rule judges, with the walls beside it, square to it, as \
                 that capability reads them.",
                "Jede Seite der Mittellinie des kleinsten umschließenden Rechtecks des \
                 Grundrisses, die eine `centre-line-distance`-Regel prüft, mit den Wänden \
                 daneben, rechtwinklig zu ihr, wie diese Fähigkeit sie liest.",
            ),
        },
        fields: &[
            field(
                "label",
                MemberFieldKind::Text,
                &en_de("Side", "Seite"),
                &en_de(
                    "How the side is named: `centre line to the left`, `… beside the +first \
                     side`, `… to the nearest wall`.",
                    "Wie die Seite benannt ist: `centre line to the left`, `… beside the \
                     +first side`, `… to the nearest wall`.",
                ),
            ),
            field(
                "distance",
                LENGTH,
                &en_de("Distance", "Abstand"),
                &en_de(
                    "From every wall that may lie beside the side to the nearest wall surely \
                     there, or just past `reach` without one; `null` where no wall may lie \
                     within `reach`.",
                    "Von jeder Wand, die neben der Seite liegen kann, bis zur nächsten sicher \
                     dort liegenden, oder knapp jenseits von `reach` ohne eine; `null`, wo \
                     keine Wand innerhalb von `reach` liegen kann.",
                ),
            ),
            field(
                "lower",
                LENGTH,
                &en_de("Least distance", "Kleinster Abstand"),
                &en_de(
                    "The least distance of any wall that may lie beside the side; `null` \
                     where none may.",
                    "Der kleinste Abstand einer Wand, die neben der Seite liegen kann; \
                     `null`, wo keine liegen kann.",
                ),
            ),
            field(
                "sure",
                LENGTH,
                &en_de("Nearest sure wall", "Nächste sichere Wand"),
                &en_de(
                    "The distance of the nearest wall surely beside the side; `null` where \
                     none surely is.",
                    "Der Abstand der nächsten Wand, die sicher neben der Seite liegt; \
                     `null`, wo keine sicher liegt.",
                ),
            ),
            field(
                "wall",
                MemberFieldKind::Objects,
                &en_de("Wall", "Wand"),
                &en_de(
                    "The nearest wall surely beside the side, if any.",
                    "Die nächste Wand, die sicher neben der Seite liegt, falls eine.",
                ),
            ),
        ],
    },
    clash::CLASH_MATRIX_PAIRS,
    clash::CLASH_PAIRS,
    walking::CLEAR_WIDTHS,
    searches::CLEARANCE_CHECKS,
    walking::CLEARANCES,
    searches::COMPOSITIONS,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "connected_spaces",
            parameters: &[
                MeasuredParameter {
                    key: "host_path",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the element to its host walls.",
                        "Die Beziehungsschritte vom Element zu seinen Wirtswänden.",
                    ),
                },
                MeasuredParameter {
                    key: "host_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The walls: source kinds, `,`-separated, or `@` a selector parameter \
                         of the rule, whose undecided objects may be walls.",
                        "Die Wände: Quellarten, durch `,` getrennt, oder mit `@` ein \
                         Selektorparameter der Regel, dessen unentschiedene Objekte Wände \
                         sein können.",
                    ),
                },
                MeasuredParameter {
                    key: "external_property",
                    kind: MeasuredParameterKind::Property,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The boolean property a wall declares itself external with.",
                        "Die boolesche Eigenschaft, mit der eine Wand sich als außenliegend \
                         angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "space_path",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the element to its spaces: one the model \
                         states, or `axioval:derived.adjacent-space` alone, forward.",
                        "Die Beziehungsschritte vom Element zu seinen Räumen: eine, die das \
                         Modell angibt, oder allein `axioval:derived.adjacent-space`, \
                         vorwärts.",
                    ),
                },
                MeasuredParameter {
                    key: "space_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The spaces; every object without it.",
                        "Die Räume; ohne Angabe jedes Objekt.",
                    ),
                },
            ],
            dimension: None,
            services: &["relationship-selection", "property-resolution"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "a host wall is undecided, reached by none, or declares no exposure",
                "the host walls disagree on their exposure",
                "the adjacency evidence records no side for a space, or the other side \
                 enters an object outside the space selection",
            ],
            label: &en_de("Connected spaces", "Verbundene Räume"),
            help: &en_de(
                "One item: the element's host walls and their exposure, how many spaces it \
                 relates to against how many the exposure needs, and, for the derived \
                 adjacency, whether they lie on the faces it needs, as `opening-spaces` \
                 reads them.",
                "Ein Element: die Wirtswände des Elements und ihre Lage, auf wie viele Räume \
                 es sich bezieht gegenüber wie vielen die Lage verlangt, und, bei der \
                 abgeleiteten Nachbarschaft, ob sie auf den verlangten Seiten liegen, wie \
                 `opening-spaces` sie liest.",
            ),
        },
        fields: &[
            field(
                "relates",
                MemberFieldKind::Text,
                &en_de("Relates", "Bezug"),
                &en_de(
                    "How many spaces it surely relates to, and through what: `relates to 1 \
                     space(s) via …`.",
                    "Auf wie viele Räume es sich sicher bezieht, und wodurch: `relates to 1 \
                     space(s) via …`.",
                ),
            ),
            field(
                "count",
                RATIO,
                &en_de("Spaces", "Räume"),
                &en_de(
                    "How many spaces it surely relates to.",
                    "Auf wie viele Räume es sich sicher bezieht.",
                ),
            ),
            field(
                "possible",
                RATIO,
                &en_de("Possible spaces", "Mögliche Räume"),
                &en_de(
                    "How many spaces it may relate to: those it surely does and those that \
                     may be spaces.",
                    "Auf wie viele Räume es sich beziehen kann: die sicheren und die, die \
                     Räume sein können.",
                ),
            ),
            field(
                "expected",
                RATIO,
                &en_de("Needed spaces", "Verlangte Räume"),
                &en_de(
                    "How many spaces its host's exposure needs: one external, two internal.",
                    "Wie viele Räume die Lage seiner Wirtswand verlangt: außen einen, innen \
                     zwei.",
                ),
            ),
            field(
                "known",
                TRUTH,
                &en_de("Decided", "Entschieden"),
                &en_de(
                    "Whether every object it relates to is surely a space or not; undecided \
                     otherwise.",
                    "Ob jedes Objekt, auf das es sich bezieht, sicher ein Raum ist oder \
                     nicht; sonst unentschieden.",
                ),
            ),
            field(
                "sides",
                TRUTH,
                &en_de("On its faces", "Auf seinen Seiten"),
                &en_de(
                    "Whether the derived adjacency places its spaces on the faces its \
                     exposure needs; true where it is not read or not the derived adjacency.",
                    "Ob die abgeleitete Nachbarschaft seine Räume auf die verlangten Seiten \
                     legt; wahr, wo sie nicht gelesen wird oder keine abgeleitete \
                     Nachbarschaft ist.",
                ),
            ),
            field(
                "placed",
                MemberFieldKind::Text,
                &en_de("Placement", "Lage"),
                &en_de(
                    "Where the faces place its spaces, where they fail; empty otherwise.",
                    "Wo die Seiten seine Räume hinlegen, wo sie scheitern; sonst leer.",
                ),
            ),
            field(
                "wall",
                MemberFieldKind::Text,
                &en_de("Wall", "Wand"),
                &en_de(
                    "`an internal wall` or `an external wall`.",
                    "`an internal wall` oder `an external wall`.",
                ),
            ),
            field(
                "hosts",
                MemberFieldKind::Text,
                &en_de("Host walls", "Wirtswände"),
                &en_de(
                    "The host walls' local ids, `,`-separated.",
                    "Die lokalen Kennungen der Wirtswände, durch `,` getrennt.",
                ),
            ),
            field(
                "requirement",
                MemberFieldKind::Text,
                &en_de("Requirement", "Anforderung"),
                &en_de(
                    "What the exposure needs, in words.",
                    "Was die Lage verlangt, in Worten.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "The host walls and the spaces it surely relates to.",
                    "Die Wirtswände und die Räume, auf die es sich sicher bezieht.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "containment_counts",
            parameters: &[
                MeasuredParameter {
                    key: "counterparts",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The outer elements (`@counterparts`).",
                        "Die äußeren Elemente (`@counterparts`).",
                    ),
                },
                MeasuredParameter {
                    key: "minimum_volume_ratio",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: true,
                    default: None,
                    help: &en_de(
                        "The share of the smaller body's volume an inner element must share.",
                        "Der Anteil am Volumen des kleineren Körpers, den ein inneres Element teilen muss.",
                    ),
                },
                MeasuredParameter {
                    key: "combine_adjacent",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether outer elements whose surfaces meet are taken together.",
                        "Ob äußere Elemente, deren Oberflächen sich berühren, zusammen genommen werden.",
                    ),
                },
                MeasuredParameter {
                    key: "cover",
                    kind: MeasuredParameterKind::Table,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The cover bands, as the rule states them (`@cover`).",
                        "Die Überdeckungsbänder, wie die Regel sie angibt (`@cover`).",
                    ),
                },
                MeasuredParameter {
                    key: "minimum_count",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The fewest inner elements an outer element may hold.",
                        "Die wenigsten inneren Elemente, die ein äußeres Element halten darf.",
                    ),
                },
                MeasuredParameter {
                    key: "maximum_count",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The most inner elements an outer element may hold.",
                        "Die meisten inneren Elemente, die ein äußeres Element halten darf.",
                    ),
                },
                MeasuredParameter {
                    key: "report_orphans",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether an inner element lying in none is reported.",
                        "Ob ein inneres Element, das in keinem liegt, gemeldet wird.",
                    ),
                },
                MeasuredParameter {
                    key: "selection",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The rule's inner elements (`@selection`).",
                        "Die inneren Elemente der Regel (`@selection`).",
                    ),
                },
            ],
            dimension: None,
            services: &["proximity"],
            exactness: MeasuredExactness::Stated,
            subject: MeasuredSubject::Project,
            not_evaluated: &["the proximity service is not registered"],
            label: &en_de("Containment counts", "Enthaltensein-Zählungen"),
            help: &en_de(
                "How many inner elements each outer element holds, from sure to possible, and the \
                 objects `containment` leaves open beyond its inner elements.",
                "Wie viele innere Elemente jedes äußere Element hält, von sicher bis möglich, und \
                 die Objekte, die `containment` über seine inneren Elemente hinaus offen lässt.",
            ),
        },
        fields: &[
            field(
                "count_checked",
                TRUTH,
                &en_de("Count", "Zählung"),
                &en_de(
                    "True on the item of an outer element's count.",
                    "Wahr am Element der Zählung eines äußeren Elements.",
                ),
            ),
            field(
                "object",
                MemberFieldKind::Objects,
                &en_de("Object", "Objekt"),
                &en_de(
                    "The object the item is on.",
                    "Das Objekt, auf dem das Element steht.",
                ),
            ),
            field(
                "count",
                RATIO,
                &en_de("Held", "Gehalten"),
                &en_de(
                    "From the inner elements it surely holds to every one it may hold.",
                    "Von den inneren Elementen, die es sicher hält, bis zu jedem, das es halten kann.",
                ),
            ),
            field(
                "held",
                RATIO,
                &en_de("Held", "Gehalten"),
                &en_de(
                    "The inner elements it surely holds.",
                    "Die inneren Elemente, die es sicher hält.",
                ),
            ),
            field(
                "may",
                RATIO,
                &en_de("May hold", "Kann halten"),
                &en_de(
                    "Every inner element it may hold.",
                    "Jedes innere Element, das es halten kann.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Held", "Gehalten"),
                &en_de(
                    "The inner elements it surely holds.",
                    "Die inneren Elemente, die es sicher hält.",
                ),
            ),
            field(
                "open_checked",
                TRUTH,
                &en_de("Open", "Offen"),
                &en_de(
                    "True on the item of an object left open.",
                    "Wahr am Element eines offen gelassenen Objekts.",
                ),
            ),
            field(
                "open",
                TRUTH,
                &en_de("Why", "Warum"),
                &en_de(
                    "Refused for why the object is open.",
                    "Verweigert mit dem Grund, warum das Objekt offen ist.",
                ),
            ),
            field(
                "reason",
                MemberFieldKind::Text,
                &en_de("Reason", "Grund"),
                &en_de(
                    "Why the item is undecided where not for incomplete evidence, as a report writes it (`missing_service`).",
                    "Warum das Element unentschieden ist, wo nicht wegen unvollständiger Belege, wie ein Bericht es schreibt (`missing_service`).",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "containment_items",
            parameters: &[
                MeasuredParameter {
                    key: "counterparts",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The outer elements (`@counterparts`).",
                        "Die äußeren Elemente (`@counterparts`).",
                    ),
                },
                MeasuredParameter {
                    key: "minimum_volume_ratio",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: true,
                    default: None,
                    help: &en_de(
                        "The share of the smaller body's volume an inner element must share.",
                        "Der Anteil am Volumen des kleineren Körpers, den ein inneres Element teilen muss.",
                    ),
                },
                MeasuredParameter {
                    key: "combine_adjacent",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether outer elements whose surfaces meet are taken together.",
                        "Ob äußere Elemente, deren Oberflächen sich berühren, zusammen genommen werden.",
                    ),
                },
                MeasuredParameter {
                    key: "cover",
                    kind: MeasuredParameterKind::Table,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The cover bands, as the rule states them (`@cover`).",
                        "Die Überdeckungsbänder, wie die Regel sie angibt (`@cover`).",
                    ),
                },
                MeasuredParameter {
                    key: "minimum_count",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The fewest inner elements an outer element may hold.",
                        "Die wenigsten inneren Elemente, die ein äußeres Element halten darf.",
                    ),
                },
                MeasuredParameter {
                    key: "maximum_count",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The most inner elements an outer element may hold.",
                        "Die meisten inneren Elemente, die ein äußeres Element halten darf.",
                    ),
                },
                MeasuredParameter {
                    key: "report_orphans",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether an inner element lying in none is reported.",
                        "Ob ein inneres Element, das in keinem liegt, gemeldet wird.",
                    ),
                },
                MeasuredParameter {
                    key: "selection",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The rule's inner elements (`@selection`).",
                        "Die inneren Elemente der Regel (`@selection`).",
                    ),
                },
            ],
            dimension: None,
            services: &["proximity"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "the proximity service is not registered",
                "the inner element's extent cannot be read",
            ],
            label: &en_de("Containment", "Enthaltensein"),
            help: &en_de(
                "What `containment` reads of an inner element: whether it lies in none, each \
                 cover distance with its band, and why something of it is not checked.",
                "Was `containment` an einem inneren Element liest: ob es in keinem liegt, jeden \
                 Überdeckungsabstand mit seinem Band und warum etwas davon nicht geprüft wird.",
            ),
        },
        fields: &[
            field(
                "orphan_checked",
                TRUTH,
                &en_de("Orphan", "Verwaist"),
                &en_de(
                    "True on the item of whether it lies in none.",
                    "Wahr am Element, ob es in keinem liegt.",
                ),
            ),
            field(
                "orphan",
                TRUTH,
                &en_de("In none", "In keinem"),
                &en_de(
                    "Whether it lies in no outer element; refused for why that is undecided.",
                    "Ob es in keinem äußeren Element liegt; verweigert mit dem Grund, warum das offen ist.",
                ),
            ),
            field(
                "orphan_words",
                MemberFieldKind::Text,
                &en_de("Words", "Worte"),
                &en_de(
                    "The finding that it lies in none, worded.",
                    "Der Befund, dass es in keinem liegt, in Worten.",
                ),
            ),
            field(
                "band_checked",
                TRUTH,
                &en_de("Band", "Band"),
                &en_de(
                    "True on the item of a cover band.",
                    "Wahr am Element eines Überdeckungsbands.",
                ),
            ),
            field(
                "distance",
                LENGTH,
                &en_de("Distance", "Abstand"),
                &en_de(
                    "The distance the band bounds: a cover, or a protrusion; refused for why it could not be measured.",
                    "Der Abstand, den das Band begrenzt: eine Überdeckung oder ein Überstand; verweigert mit dem Grund, warum er nicht gemessen werden konnte.",
                ),
            ),
            field(
                "low",
                LENGTH,
                &en_de("Least", "Mindestens"),
                &en_de(
                    "The band's least distance, if any.",
                    "Der kleinste Abstand des Bands, falls angegeben.",
                ),
            ),
            field(
                "high",
                LENGTH,
                &en_de("Most", "Höchstens"),
                &en_de(
                    "The band's greatest distance, if any.",
                    "Der größte Abstand des Bands, falls angegeben.",
                ),
            ),
            field(
                "below_words",
                MemberFieldKind::Text,
                &en_de("Below", "Darunter"),
                &en_de(
                    "The finding below the least distance, worded.",
                    "Der Befund unter dem kleinsten Abstand, in Worten.",
                ),
            ),
            field(
                "above_words",
                MemberFieldKind::Text,
                &en_de("Above", "Darüber"),
                &en_de(
                    "The finding above the greatest distance, worded.",
                    "Der Befund über dem größten Abstand, in Worten.",
                ),
            ),
            field(
                "straddle_words",
                MemberFieldKind::Text,
                &en_de("Straddles", "Überspannt"),
                &en_de(
                    "Why the band cannot be judged, worded.",
                    "Warum das Band nicht beurteilt werden kann, in Worten.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Outer", "Äußeres"),
                &en_de(
                    "The outer element the band is measured to.",
                    "Das äußere Element, zu dem das Band gemessen wird.",
                ),
            ),
            field(
                "open_checked",
                TRUTH,
                &en_de("Open", "Offen"),
                &en_de(
                    "True on the item of something not checked.",
                    "Wahr am Element von etwas nicht Geprüftem.",
                ),
            ),
            field(
                "open",
                TRUTH,
                &en_de("Why", "Warum"),
                &en_de(
                    "Refused for why something of it is not checked.",
                    "Verweigert mit dem Grund, warum etwas davon nicht geprüft wird.",
                ),
            ),
            field(
                "reason",
                MemberFieldKind::Text,
                &en_de("Reason", "Grund"),
                &en_de(
                    "Why the item is undecided where not for incomplete evidence, as a report writes it (`missing_service`).",
                    "Warum das Element unentschieden ist, wo nicht wegen unvollständiger Belege, wie ein Bericht es schreibt (`missing_service`).",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "coordinate_differences",
            parameters: &COORDINATE_DIFFERENCES,
            dimension: None,
            services: COORDINATES,
            exactness: MeasuredExactness::Stated,
            subject: MeasuredSubject::Source,
            not_evaluated: &[COORDINATES_UNREAD],
            label: &en_de("Coordinate differences", "Koordinatenabweichungen"),
            help: &en_de(
                "Each statement in which the source's coordinate system departs from the \
                 reference source's beyond the tolerances, or cannot be compared with it, \
                 as `coordinate-consistency` compares them: the world frame, true north, \
                 the map conversion and the site, then the georeference. The reference \
                 itself lists only a missing map conversion the rule requires.",
                "Jede Angabe, in der das Koordinatensystem der Quelle über die Toleranzen \
                 hinaus von dem der Referenzquelle abweicht oder nicht mit ihm verglichen \
                 werden kann, wie `coordinate-consistency` sie vergleicht: der Weltrahmen, \
                 geografisch Nord, die Kartenumrechnung und das Grundstück, dann die \
                 Georeferenz. Die Referenz selbst nennt nur eine fehlende geforderte \
                 Kartenumrechnung.",
            ),
        },
        fields: &[
            field(
                "found",
                MemberFieldKind::Truth,
                &en_de("Differs", "Weicht ab"),
                &en_de(
                    "True where the statement differs beyond the tolerances; undecided, \
                     with why, where it cannot be compared.",
                    "Wahr, wo die Angabe über die Toleranzen hinaus abweicht; \
                     unentschieden, mit Grund, wo sie nicht verglichen werden kann.",
                ),
            ),
            field(
                "finding",
                MemberFieldKind::Text,
                &en_de("Difference", "Abweichung"),
                &en_de(
                    "The difference in words (`map offset moved by 1.0000 m`), or why the \
                     statement cannot be compared.",
                    "Die Abweichung in Worten (`map offset moved by 1.0000 m`) oder warum \
                     die Angabe nicht verglichen werden kann.",
                ),
            ),
            field(
                "recorded",
                MemberFieldKind::Truth,
                &en_de("Recorded", "Erfasst"),
                &en_de(
                    "False where the statement cannot be compared because a source records \
                     none (a map conversion), true otherwise.",
                    "Falsch, wo die Angabe nicht verglichen werden kann, weil eine Quelle \
                     keine erfasst (eine Kartenumrechnung), sonst wahr.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "corridor_end_openings",
            parameters: &[
                MeasuredParameter {
                    key: "path",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the corridor to its openings.",
                        "Die Beziehungsschritte vom Flur zu seinen Öffnungen.",
                    ),
                },
                MeasuredParameter {
                    key: "openings",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The openings searched: source kinds, `,`-separated, or `@` a selector \
                         parameter of the rule, whose undecided objects are searched too.",
                        "Die gesuchten Öffnungen: Quellarten, durch `,` getrennt, oder mit `@` \
                         ein Selektorparameter der Regel, dessen unentschiedene Objekte \
                         ebenfalls gesucht werden.",
                    ),
                },
                MeasuredParameter {
                    key: "depth",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far behind an end wall's face an opening may lie, in metres; half \
                         a metre without it.",
                        "Wie weit hinter der Ansicht einer Endwand eine Öffnung liegen darf, in \
                         Metern; ohne Angabe ein halber Meter.",
                    ),
                },
                MeasuredParameter {
                    key: "facing",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How much of an end wall an opening must face, more than this many \
                         metres; a tenth of a metre without it.",
                        "Wie viel einer Endwand eine Öffnung zugewandt sein muss, mehr als so \
                         viele Meter; ohne Angabe ein Zehntelmeter.",
                    ),
                },
            ],
            dimension: None,
            services: &["plan-span", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            not_evaluated: &[
                "the corridor ends cannot be measured",
                "the path from the corridor cannot be followed",
            ],
            label: &en_de("Openings at a corridor's ends", "Öffnungen an Flurenden"),
            help: &en_de(
                "The openings a corridor reaches, each searched against the walls its ends \
                 run into, as `corridor-end-openings` searches them: whether it sits in one, \
                 which, and whether the selection picks it.",
                "Die Öffnungen, die ein Flur erreicht, je gegen die Wände gesucht, auf die \
                 seine Enden treffen, wie `corridor-end-openings` sucht: ob sie in einer \
                 liegt, in welcher, und ob die Auswahl sie trifft.",
            ),
            subject: MeasuredSubject::Object,
        },
        fields: &[
            field(
                "opening",
                MemberFieldKind::Objects,
                &en_de("Opening", "Öffnung"),
                &en_de("The opening searched.", "Die gesuchte Öffnung."),
            ),
            field(
                "corridor",
                MemberFieldKind::Objects,
                &en_de("Corridor", "Flur"),
                &en_de(
                    "The corridor searched from.",
                    "Der Flur, von dem aus gesucht wird.",
                ),
            ),
            field(
                "picked",
                TRUTH,
                &en_de("Selected", "Ausgewählt"),
                &en_de(
                    "Whether the selection surely picks the opening; undecided where it \
                     cannot tell.",
                    "Ob die Auswahl die Öffnung sicher trifft; unentschieden, wo sie es nicht \
                     entscheidet.",
                ),
            ),
            field(
                "unpicked",
                MemberFieldKind::Text,
                &en_de("Why undecided", "Warum unentschieden"),
                &en_de(
                    "Why the selection cannot decide the opening; empty where it can.",
                    "Warum die Auswahl die Öffnung nicht entscheidet; leer, wo sie es kann.",
                ),
            ),
            field(
                "sits",
                TRUTH,
                &en_de("Sits in an end wall", "Liegt in einer Endwand"),
                &en_de(
                    "Whether the opening sits in a wall the corridor ends at; undecided \
                     where an end wall, or the opening's contact with one, cannot decide it.",
                    "Ob die Öffnung in einer Wand liegt, an der der Flur endet; \
                     unentschieden, wo eine Endwand oder ihr Kontakt damit es nicht \
                     entscheidet.",
                ),
            ),
            field(
                "walls",
                MemberFieldKind::Text,
                &en_de("End walls", "Endwände"),
                &en_de(
                    "The end walls it sits in, with its gap to each and how much it faces.",
                    "Die Endwände, in denen sie liegt, mit Abstand und zugewandter Länge.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "distance_items",
            parameters: &[
                MeasuredParameter {
                    key: "counterparts",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The counterparts each subject keeps its distance from (`@counterparts`).",
                        "Die Gegenstücke, zu denen jedes Subjekt seinen Abstand hält (`@counterparts`).",
                    ),
                },
                MeasuredParameter {
                    key: "minimum_metres",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "No counterpart may come closer, in metres, if stated.",
                        "Kein Gegenstück darf näher kommen, in Metern, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "maximum_metres",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "A counterpart must lie within it, in metres, if stated.",
                        "Ein Gegenstück muss innerhalb liegen, in Metern, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "mode",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "`nearest`, `none_closer_than` or `at_least`, as the rule states it.",
                        "`nearest`, `none_closer_than` oder `at_least`, wie die Regel es angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "count",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How many counterparts `at_least` requires.",
                        "Wie viele Gegenstücke `at_least` verlangt.",
                    ),
                },
                MeasuredParameter {
                    key: "projection",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The projection distances are measured in, as the rule states it.",
                        "Die Projektion, in der Abstände gemessen werden, wie die Regel sie angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "footprint_offset_metres",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far a vertical distance reaches beyond the footprint.",
                        "Wie weit ein vertikaler Abstand über die Grundfläche hinausreicht.",
                    ),
                },
                MeasuredParameter {
                    key: "vertical_direction",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Which side a vertical distance counts, as the rule states it.",
                        "Welche Seite ein vertikaler Abstand zählt, wie die Regel sie angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "subject_extent",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "What a subject is measured by, as the rule states it.",
                        "Woran ein Subjekt gemessen wird, wie die Regel es angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "counterpart_extent",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "What a counterpart is measured by, as the rule states it.",
                        "Woran ein Gegenstück gemessen wird, wie die Regel es angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "subject_surface",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The subject's surface a vertical distance runs from.",
                        "Die Fläche des Subjekts, von der ein vertikaler Abstand ausgeht.",
                    ),
                },
                MeasuredParameter {
                    key: "counterpart_surface",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The counterpart's surface a vertical distance runs to.",
                        "Die Fläche des Gegenstücks, zu der ein vertikaler Abstand reicht.",
                    ),
                },
                MeasuredParameter {
                    key: "elevation_overlap",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether only counterparts at the subject's heights count.",
                        "Ob nur Gegenstücke auf der Höhe des Subjekts zählen.",
                    ),
                },
                MeasuredParameter {
                    key: "elevation_offset_metres",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The height gap a counterpart must stay under.",
                        "Der Höhenabstand, unter dem ein Gegenstück bleiben muss.",
                    ),
                },
                MeasuredParameter {
                    key: "container_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The containers that count (`@container_selector`).",
                        "Die Behälter, die zählen (`@container_selector`).",
                    ),
                },
                MeasuredParameter {
                    key: "relationship",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The relationship to the containers, as the rule states it.",
                        "Die Beziehung zu den Behältern, wie die Regel sie angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "direction",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The direction the relationship is followed in.",
                        "Die Richtung, in der der Beziehung gefolgt wird.",
                    ),
                },
                MeasuredParameter {
                    key: "follow_chain",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether chains of the relationship are followed.",
                        "Ob Ketten der Beziehung verfolgt werden.",
                    ),
                },
                MeasuredParameter {
                    key: "path",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The path to the containers, step by step.",
                        "Der Pfad zu den Behältern, Schritt für Schritt.",
                    ),
                },
                MeasuredParameter {
                    key: "skip_absent_relationship_ends",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether a relationship end that is absent is skipped.",
                        "Ob ein fehlendes Beziehungsende übersprungen wird.",
                    ),
                },
                MeasuredParameter {
                    key: "selection",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The rule's subjects (`@selection`).",
                        "Die Subjekte der Regel (`@selection`).",
                    ),
                },
            ],
            dimension: None,
            services: &["proximity", "object-frame", "vertical-extent"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "a service is not registered",
                "the subject's extent, heights or containers cannot be read",
            ],
            label: &en_de("Distance", "Abstand"),
            help: &en_de(
                "What `distance` judges of a subject: the nearest counterpart closer than the \
                 minimum, the nearest one beyond the maximum, or the counterparts within the \
                 range, with the capability's words.",
                "Was `distance` an einem Subjekt beurteilt: das nächste Gegenstück näher als das \
                 Minimum, das nächste jenseits des Maximums oder die Gegenstücke im Bereich, mit \
                 den Worten der Fähigkeit.",
            ),
        },
        fields: &[
            field(
                "apart_checked",
                TRUTH,
                &en_de("Apart", "Abstand"),
                &en_de(
                    "Whether no counterpart may come closer than the minimum.",
                    "Ob kein Gegenstück näher als das Minimum kommen darf.",
                ),
            ),
            field(
                "apart",
                LENGTH,
                &en_de("Nearest closer", "Nächstes näher"),
                &en_de(
                    "The nearest counterpart closer than the minimum; `null` where none surely is; refused for why it is open.",
                    "Das nächste Gegenstück näher als das Minimum; `null`, wo sicher keines ist; verweigert mit dem Grund, warum es offen ist.",
                ),
            ),
            field(
                "apart_words",
                MemberFieldKind::Text,
                &en_de("Words", "Worte"),
                &en_de(
                    "The finding of a counterpart too close, worded.",
                    "Der Befund eines zu nahen Gegenstücks, in Worten.",
                ),
            ),
            field(
                "apart_related",
                MemberFieldKind::Objects,
                &en_de("Too close", "Zu nah"),
                &en_de("The counterparts too close.", "Die zu nahen Gegenstücke."),
            ),
            field(
                "reach_checked",
                TRUTH,
                &en_de("Reach", "Reichweite"),
                &en_de(
                    "Whether the nearest counterpart must lie within the maximum.",
                    "Ob das nächste Gegenstück innerhalb des Maximums liegen muss.",
                ),
            ),
            field(
                "reach",
                LENGTH,
                &en_de("Nearest", "Nächstes"),
                &en_de(
                    "The nearest counterpart where none lies within the maximum; `null` where one does or none is near; refused for why it is open.",
                    "Das nächste Gegenstück, wo keines innerhalb des Maximums liegt; `null`, wo eines es tut oder keines nahe ist; verweigert mit dem Grund, warum es offen ist.",
                ),
            ),
            field(
                "reach_words",
                MemberFieldKind::Text,
                &en_de("Words", "Worte"),
                &en_de(
                    "The finding of no counterpart within the maximum, worded.",
                    "Der Befund, dass kein Gegenstück innerhalb des Maximums liegt, in Worten.",
                ),
            ),
            field(
                "reach_related",
                MemberFieldKind::Objects,
                &en_de("Nearest", "Nächstes"),
                &en_de(
                    "The nearest counterpart named.",
                    "Das genannte nächste Gegenstück.",
                ),
            ),
            field(
                "none_within",
                TRUTH,
                &en_de("None within", "Keines innerhalb"),
                &en_de(
                    "Whether no counterpart lies near at all.",
                    "Ob überhaupt kein Gegenstück nahe liegt.",
                ),
            ),
            field(
                "count_checked",
                TRUTH,
                &en_de("Count", "Zählung"),
                &en_de(
                    "Whether `count` counterparts must lie within the range.",
                    "Ob `count` Gegenstücke im Bereich liegen müssen.",
                ),
            ),
            field(
                "count",
                RATIO,
                &en_de("Counted", "Gezählt"),
                &en_de(
                    "From the counterparts surely within the range to every one that may be; refused for why it is open.",
                    "Von den sicher im Bereich liegenden Gegenstücken bis zu jedem, das es kann; verweigert mit dem Grund, warum es offen ist.",
                ),
            ),
            field(
                "count_words",
                MemberFieldKind::Text,
                &en_de("Words", "Worte"),
                &en_de(
                    "The finding of too few counterparts, worded.",
                    "Der Befund zu weniger Gegenstücke, in Worten.",
                ),
            ),
            field(
                "count_related",
                MemberFieldKind::Objects,
                &en_de("Counted", "Gezählt"),
                &en_de(
                    "The counterparts that may lie within the range.",
                    "Die Gegenstücke, die im Bereich liegen können.",
                ),
            ),
            field(
                "reason",
                MemberFieldKind::Text,
                &en_de("Reason", "Grund"),
                &en_de(
                    "Why the item is undecided where not for incomplete evidence, as a report writes it (`missing_service`).",
                    "Warum das Element unentschieden ist, wo nicht wegen unvollständiger Belege, wie ein Bericht es schreibt (`missing_service`).",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "distance_open",
            parameters: &[
                MeasuredParameter {
                    key: "counterparts",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The counterparts each subject keeps its distance from (`@counterparts`).",
                        "Die Gegenstücke, zu denen jedes Subjekt seinen Abstand hält (`@counterparts`).",
                    ),
                },
                MeasuredParameter {
                    key: "minimum_metres",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "No counterpart may come closer, in metres, if stated.",
                        "Kein Gegenstück darf näher kommen, in Metern, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "maximum_metres",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "A counterpart must lie within it, in metres, if stated.",
                        "Ein Gegenstück muss innerhalb liegen, in Metern, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "mode",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "`nearest`, `none_closer_than` or `at_least`, as the rule states it.",
                        "`nearest`, `none_closer_than` oder `at_least`, wie die Regel es angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "count",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How many counterparts `at_least` requires.",
                        "Wie viele Gegenstücke `at_least` verlangt.",
                    ),
                },
                MeasuredParameter {
                    key: "projection",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The projection distances are measured in, as the rule states it.",
                        "Die Projektion, in der Abstände gemessen werden, wie die Regel sie angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "footprint_offset_metres",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far a vertical distance reaches beyond the footprint.",
                        "Wie weit ein vertikaler Abstand über die Grundfläche hinausreicht.",
                    ),
                },
                MeasuredParameter {
                    key: "vertical_direction",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Which side a vertical distance counts, as the rule states it.",
                        "Welche Seite ein vertikaler Abstand zählt, wie die Regel sie angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "subject_extent",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "What a subject is measured by, as the rule states it.",
                        "Woran ein Subjekt gemessen wird, wie die Regel es angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "counterpart_extent",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "What a counterpart is measured by, as the rule states it.",
                        "Woran ein Gegenstück gemessen wird, wie die Regel es angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "subject_surface",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The subject's surface a vertical distance runs from.",
                        "Die Fläche des Subjekts, von der ein vertikaler Abstand ausgeht.",
                    ),
                },
                MeasuredParameter {
                    key: "counterpart_surface",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The counterpart's surface a vertical distance runs to.",
                        "Die Fläche des Gegenstücks, zu der ein vertikaler Abstand reicht.",
                    ),
                },
                MeasuredParameter {
                    key: "elevation_overlap",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether only counterparts at the subject's heights count.",
                        "Ob nur Gegenstücke auf der Höhe des Subjekts zählen.",
                    ),
                },
                MeasuredParameter {
                    key: "elevation_offset_metres",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The height gap a counterpart must stay under.",
                        "Der Höhenabstand, unter dem ein Gegenstück bleiben muss.",
                    ),
                },
                MeasuredParameter {
                    key: "container_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The containers that count (`@container_selector`).",
                        "Die Behälter, die zählen (`@container_selector`).",
                    ),
                },
                MeasuredParameter {
                    key: "relationship",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The relationship to the containers, as the rule states it.",
                        "Die Beziehung zu den Behältern, wie die Regel sie angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "direction",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The direction the relationship is followed in.",
                        "Die Richtung, in der der Beziehung gefolgt wird.",
                    ),
                },
                MeasuredParameter {
                    key: "follow_chain",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether chains of the relationship are followed.",
                        "Ob Ketten der Beziehung verfolgt werden.",
                    ),
                },
                MeasuredParameter {
                    key: "path",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The path to the containers, step by step.",
                        "Der Pfad zu den Behältern, Schritt für Schritt.",
                    ),
                },
                MeasuredParameter {
                    key: "skip_absent_relationship_ends",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether a relationship end that is absent is skipped.",
                        "Ob ein fehlendes Beziehungsende übersprungen wird.",
                    ),
                },
                MeasuredParameter {
                    key: "selection",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The rule's subjects (`@selection`).",
                        "Die Subjekte der Regel (`@selection`).",
                    ),
                },
            ],
            dimension: None,
            services: &["proximity", "object-frame", "vertical-extent"],
            exactness: MeasuredExactness::Stated,
            subject: MeasuredSubject::Project,
            not_evaluated: &["a service is not registered"],
            label: &en_de("Distance open", "Abstand offen"),
            help: &en_de(
                "The objects `distance` leaves open beyond its subjects: those its selections \
                 cannot decide and the counterparts whose extent cannot be read.",
                "Die Objekte, die `distance` über seine Subjekte hinaus offen lässt: die, die seine \
                 Auswahlen nicht entscheiden, und die Gegenstücke, deren Ausdehnung nicht gelesen \
                 werden kann.",
            ),
        },
        fields: &[
            field(
                "object",
                MemberFieldKind::Objects,
                &en_de("Object", "Objekt"),
                &en_de("The object left open.", "Das offen gelassene Objekt."),
            ),
            field(
                "open",
                TRUTH,
                &en_de("Open", "Offen"),
                &en_de(
                    "Refused for why the object is open.",
                    "Verweigert mit dem Grund, warum das Objekt offen ist.",
                ),
            ),
            field(
                "reason",
                MemberFieldKind::Text,
                &en_de("Reason", "Grund"),
                &en_de(
                    "Why the item is undecided where not for incomplete evidence, as a report writes it (`missing_service`).",
                    "Warum das Element unentschieden ist, wo nicht wegen unvollständiger Belege, wie ein Bericht es schreibt (`missing_service`).",
                ),
            ),
        ],
    },
    searches::DISTANCE_ROWS,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "effective_missing",
            parameters: EFFECTIVE,
            dimension: None,
            services: EFFECT_SERVICES,
            exactness: MeasuredExactness::Stated,
            subject: MeasuredSubject::Object,
            not_evaluated: EFFECT_UNMEASURED,
            label: &en_de("Missing capacities", "Fehlende Kapazitäten"),
            help: &en_de(
                "Each source surely contributing to `effective_capacity` that states no \
                 capacity or multiplier (absent, null or blank).",
                "Jede sicher zu `effective_capacity` beitragende Quelle, die keine Kapazität \
                 oder keinen Faktor angibt (fehlend, null oder leer).",
            ),
        },
        fields: &[
            field(
                "source",
                MemberFieldKind::Objects,
                &en_de("Source", "Quelle"),
                &en_de(
                    "The source stating nothing.",
                    "Die Quelle, die nichts angibt.",
                ),
            ),
            field(
                "property",
                MemberFieldKind::Text,
                &en_de("Property", "Eigenschaft"),
                &en_de(
                    "The property it states nothing under, as the rule names it.",
                    "Die Eigenschaft, unter der sie nichts angibt, wie die Regel sie nennt.",
                ),
            ),
            field(
                "missing",
                MemberFieldKind::Truth,
                &en_de("Missing", "Fehlend"),
                &en_de(
                    "Always true: the value is missing.",
                    "Immer wahr: der Wert fehlt.",
                ),
            ),
        ],
    },
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
            subject: MeasuredSubject::Object,
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
            subject: MeasuredSubject::Object,
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
            name: "exit_separation",
            parameters: &[
                MeasuredParameter {
                    key: "exit_path",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the space to its exits.",
                        "Die Beziehungsschritte vom Raum zu seinen Ausgängen.",
                    ),
                },
                MeasuredParameter {
                    key: "exit_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The objects that may be exits; those the selection leaves undecided possible exits.",
                        "Die Objekte, die Ausgänge sein können; die unentschiedenen mögliche Ausgänge.",
                    ),
                },
                MeasuredParameter {
                    key: "fraction",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The share of the longest plan diagonal the exits must lie apart; half without it.",
                        "Der Anteil der längsten Grundrissdiagonale, den Ausgänge auseinanderliegen müssen; ohne Angabe die Hälfte.",
                    ),
                },
                MeasuredParameter {
                    key: "flag",
                    kind: MeasuredParameterKind::Property,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The property choosing the second share where it is true.",
                        "Die Eigenschaft, die den zweiten Anteil wählt, wenn sie wahr ist.",
                    ),
                },
                MeasuredParameter {
                    key: "flag_path",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The relationship steps to the objects `flag` is read on.",
                        "Die Beziehungsschritte zu den Objekten, an denen `flag` gelesen wird.",
                    ),
                },
                MeasuredParameter {
                    key: "flagged_fraction",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The share where the flag is true.",
                        "Der Anteil, wenn die Eigenschaft wahr ist.",
                    ),
                },
                MeasuredParameter {
                    key: "flag_sources",
                    kind: MeasuredParameterKind::Table,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The places the flag is read from, in order.",
                        "Die Orte, an denen die Eigenschaft der Reihe nach gelesen wird.",
                    ),
                },
                MeasuredParameter {
                    key: "flag_default",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The flag's value where no place states one.",
                        "Der Wert der Eigenschaft, wo kein Ort einen angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "separation",
                    kind: MeasuredParameterKind::Choice {
                        options: &["closest", "centres", "farthest"],
                    },
                    required: false,
                    default: None,
                    help: &en_de(
                        "Between the exits' closest points, centres or farthest points in plan; the closest without it.",
                        "Zwischen den nächsten Punkten, Mittelpunkten oder fernsten Punkten der Ausgänge im Grundriss; ohne Angabe den nächsten.",
                    ),
                },
                MeasuredParameter {
                    key: "pairs",
                    kind: MeasuredParameterKind::Choice {
                        options: &["any", "all"],
                    },
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether some pair or every pair must lie far enough apart; some without it.",
                        "Ob irgendein Paar oder jedes Paar weit genug auseinanderliegen muss; ohne Angabe irgendeines.",
                    ),
                },
                MeasuredParameter {
                    key: "minimum_exits",
                    kind: MeasuredParameterKind::Number { minimum: 1.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How many exits the space needs at least.",
                        "Wie viele Ausgänge der Raum mindestens haben muss.",
                    ),
                },
            ],
            dimension: None,
            services: &["plan-span", "proximity", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "the path to the exits cannot be read",
                "a service is not registered",
                "the longest plan diagonal cannot be measured",
            ],
            label: &en_de("Exit separation", "Ausgangsabstand"),
            help: &en_de(
                "How far apart a space's exits lie against the share of its longest plan \
                 diagonal they must, as `exit-separation` measures it: an item counting the \
                 exits where a minimum is declared, and one with the separation of the \
                 pairs.",
                "Wie weit die Ausgänge eines Raums auseinanderliegen, gegen den Anteil seiner \
                 längsten Grundrissdiagonale, den sie müssen, wie `exit-separation` es misst: \
                 ein Element, das die Ausgänge zählt, wo ein Minimum erklärt ist, und eines \
                 mit dem Abstand der Paare.",
            ),
        },
        fields: &[
            field(
                "counted",
                TRUTH,
                &en_de("Counted", "Gezählt"),
                &en_de(
                    "True on the item counting the exits (only with `minimum_exits`).",
                    "Wahr am Element, das die Ausgänge zählt (nur mit `minimum_exits`).",
                ),
            ),
            field(
                "exits",
                RATIO,
                &en_de("Exits", "Ausgänge"),
                &en_de(
                    "From the exits surely reached to every one that may be.",
                    "Von den sicheren Ausgängen bis zu jedem möglichen.",
                ),
            ),
            field(
                "sure",
                RATIO,
                &en_de("Sure exits", "Sichere Ausgänge"),
                &en_de(
                    "The exits surely reached.",
                    "Die sicher erreichten Ausgänge.",
                ),
            ),
            field(
                "possible",
                RATIO,
                &en_de("Possible exits", "Mögliche Ausgänge"),
                &en_de(
                    "Every exit that may be one.",
                    "Jeder Ausgang, der es sein kann.",
                ),
            ),
            field(
                "relation",
                MemberFieldKind::Text,
                &en_de("Relation", "Beziehung"),
                &en_de(
                    "Words naming the path to the exits.",
                    "Worte, die den Weg zu den Ausgängen nennen.",
                ),
            ),
            field(
                "undecided",
                MemberFieldKind::Text,
                &en_de("Undecided exits", "Unentschiedene Ausgänge"),
                &en_de(
                    "Why each possible exit is undecided.",
                    "Warum jeder mögliche Ausgang unentschieden ist.",
                ),
            ),
            field(
                "named",
                MemberFieldKind::Objects,
                &en_de("Exits", "Ausgänge"),
                &en_de(
                    "The sure, then the possible exits.",
                    "Die sicheren, dann die möglichen Ausgänge.",
                ),
            ),
            field(
                "separated",
                TRUTH,
                &en_de("Separated", "Getrennt"),
                &en_de(
                    "True on the item measuring how far apart the exits lie.",
                    "Wahr am Element, das den Abstand der Ausgänge misst.",
                ),
            ),
            field(
                "separation",
                LENGTH,
                &en_de("Separation", "Abstand"),
                &en_de(
                    "With `pairs` `any` the greatest, with `all` the least separation of the pairs measured; `null` where none is, undecided with fewer than two sure exits.",
                    "Mit `pairs` `any` der größte, mit `all` der kleinste Abstand der gemessenen Paare; `null`, wo keines gemessen ist, unentschieden bei weniger als zwei sicheren Ausgängen.",
                ),
            ),
            field(
                "required",
                LENGTH,
                &en_de("Required", "Erforderlich"),
                &en_de(
                    "The share of the longest diagonal; an interval over both shares where the flag is unknown.",
                    "Der Anteil der längsten Diagonale; ein Intervall über beide Anteile, wo die Eigenschaft unbekannt ist.",
                ),
            ),
            field(
                "short",
                TRUTH,
                &en_de("Too few", "Zu wenige"),
                &en_de(
                    "Whether the space surely has fewer exits than `minimum_exits`.",
                    "Ob der Raum sicher weniger Ausgänge als `minimum_exits` hat.",
                ),
            ),
            field(
                "undecided_fail",
                TRUTH,
                &en_de("Open shortfall", "Offenes Unterschreiten"),
                &en_de(
                    "Whether a shortfall stays open, pairs not measured or exits undecided.",
                    "Ob ein Unterschreiten offen bleibt, weil Paare ungemessen oder Ausgänge unentschieden sind.",
                ),
            ),
            field(
                "undecided_pass",
                TRUTH,
                &en_de("Open pass", "Offenes Bestehen"),
                &en_de(
                    "Whether a pass stays open, pairs not measured or exits undecided.",
                    "Ob ein Bestehen offen bleibt, weil Paare ungemessen oder Ausgänge unentschieden sind.",
                ),
            ),
            field(
                "open",
                MemberFieldKind::Text,
                &en_de("Open", "Offen"),
                &en_de("Why pairs are open.", "Warum Paare offen sind."),
            ),
            field(
                "requirement",
                MemberFieldKind::Text,
                &en_de("Requirement", "Anforderung"),
                &en_de(
                    "Words naming the required separation.",
                    "Worte, die den erforderlichen Abstand nennen.",
                ),
            ),
            field(
                "failed",
                MemberFieldKind::Text,
                &en_de("Too close", "Zu nah"),
                &en_de(
                    "Words naming the pairs too close.",
                    "Worte, die die Paare nennen, die zu nah liegen.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "The exits of the pairs too close and the objects the flag was read on.",
                    "Die Ausgänge der zu nahen Paare und die Objekte, an denen die Eigenschaft gelesen wurde.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: FACE_PIECES,
            parameters: &[FACE, FACING[0], FACING[1]],
            dimension: None,
            services: &["vertical-extent"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
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
    searches::FREE_FLOOR_FIT,
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
            subject: MeasuredSubject::Object,
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
            parameters: &GUARD,
            dimension: None,
            services: &["guard", "type-hierarchy"],
            exactness: MeasuredExactness::Stated,
            subject: MeasuredSubject::Object,
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
            field(
                "climbable",
                MemberFieldKind::Objects,
                &en_de("Climbable object", "Bekletterbares Objekt"),
                &en_de(
                    "The first such object no taller than `climb_height` (within a \
                     micrometre), as the guard service lists them; none without one.",
                    "Das erste solche Objekt, das nicht höher als `climb_height` ist (auf \
                     einen Mikrometer), wie der Absturzdienst sie auflistet; keines ohne.",
                ),
            ),
            field(
                "partial_height",
                LENGTH,
                &en_de("Partial barrier height", "Höhe teilweiser Absturzsicherung"),
                &en_de(
                    "The height of the tallest barrier within the gaps that runs along part \
                     of the edge at all; `null` with none.",
                    "Die Höhe der höchsten Absturzsicherung innerhalb der Lücken, die \
                     überhaupt an einem Teil der Kante verläuft; `null` ohne.",
                ),
            ),
            field(
                "tallest",
                MemberFieldKind::Objects,
                &en_de("Tallest barrier", "Höchste Absturzsicherung"),
                &en_de(
                    "The tallest barrier within `platform_gap` (the last of equally tall ones, \
                     as the guard service lists them); none without one.",
                    "Die höchste Absturzsicherung innerhalb von `platform_gap` (die letzte \
                     gleich hoher, wie der Absturzdienst sie auflistet); keine ohne.",
                ),
            ),
            field(
                "tallest_top",
                LENGTH,
                &en_de(
                    "Tallest barrier's top",
                    "Oberkante der höchsten Absturzsicherung",
                ),
                &en_de(
                    "The tallest barrier's top above the surface where it is measured from a \
                     curb under it (`measure_from` `curb`); `null` otherwise.",
                    "Die Oberkante der höchsten Absturzsicherung über der Lauffläche, wo sie \
                     von einer Aufkantung darunter gemessen wird (`measure_from` `curb`); \
                     sonst `null`.",
                ),
            ),
            field(
                "nearest_gap",
                LENGTH,
                &en_de(
                    "Nearest landing's gap",
                    "Abstand der nächsten Auftrittsfläche",
                ),
                &en_de(
                    "How far from the edge the landing nearest it lies, whatever its width or \
                     depth; `null` with no landing.",
                    "Wie weit die nächste Auftrittsfläche von der Kante liegt, gleich welcher \
                     Breite oder Tiefe; `null` ohne Auftrittsfläche.",
                ),
            ),
            field(
                "nearest_fall",
                LENGTH,
                &en_de(
                    "Fall onto the nearest landing",
                    "Fallhöhe auf die nächste Auftrittsfläche",
                ),
                &en_de(
                    "How far below the surface the nearest landing's top lies; `null` with \
                     no landing.",
                    "Wie weit die Oberkante der nächsten Auftrittsfläche unter der Lauffläche \
                     liegt; `null` ohne Auftrittsfläche.",
                ),
            ),
            field(
                "nearest_width",
                LENGTH,
                &en_de(
                    "Nearest landing's width",
                    "Breite der nächsten Auftrittsfläche",
                ),
                &en_de(
                    "How wide the nearest landing is to stand on; `null` with no landing.",
                    "Wie breit die nächste Auftrittsfläche zum Stehen ist; `null` ohne \
                     Auftrittsfläche.",
                ),
            ),
            field(
                "nearest",
                MemberFieldKind::Objects,
                &en_de("Nearest landing", "Nächste Auftrittsfläche"),
                &en_de(
                    "The landing nearest the edge (the first of equally near ones); none \
                     without one.",
                    "Die Auftrittsfläche, die der Kante am nächsten liegt (die erste gleich \
                     naher); keine ohne.",
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
            subject: MeasuredSubject::Object,
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
            name: "limit_row",
            parameters: &super::registry::LIMIT_ROWS,
            dimension: None,
            services: &["property-resolution", "relationship-selection"],
            exactness: MeasuredExactness::Stated,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "a key a row tests cannot be read",
                "rows tie for most specific",
            ],
            label: &en_de("Limit row", "Grenzzeile"),
            help: &en_de(
                "One item: the most specific row of `limits` the object's keys select, as \
                 `keyed-limit` selects it, or that none matches.",
                "Ein Element: die spezifischste Zeile von `limits`, die die Schlüssel des \
                 Objekts wählen, wie `keyed-limit` sie wählt, oder dass keine passt.",
            ),
        },
        fields: &[
            field(
                "listed",
                TRUTH,
                &en_de("Listed", "Gelistet"),
                &en_de(
                    "Whether a row matches the object's keys.",
                    "Ob eine Zeile zu den Schlüsseln des Objekts passt.",
                ),
            ),
            field(
                "row",
                RATIO,
                &en_de("Row", "Zeile"),
                &en_de(
                    "The index of the row the object's keys select, from 0; `null` where none \
                     matches.",
                    "Der Index der Zeile, die die Schlüssel des Objekts wählen, ab 0; `null`, \
                     wo keine passt.",
                ),
            ),
            field(
                "keys",
                MemberFieldKind::Text,
                &en_de("Keys", "Schlüssel"),
                &en_de(
                    "The keys as a message describes them: each property and its value.",
                    "Die Schlüssel, wie eine Meldung sie beschreibt: jede Eigenschaft und ihr \
                     Wert.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "The objects a key was read on along its path.",
                    "Die Objekte, an denen ein Schlüssel entlang seines Pfads gelesen wurde.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "limited_values",
            parameters: &super::registry::LIMITED,
            dimension: None,
            services: &[
                "property-resolution",
                "relationship-selection",
                "vertical-extent",
            ],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "a key a row tests cannot be read, or rows tie for most specific",
                "the quantity cannot be measured",
            ],
            label: &en_de("Limited values", "Begrenzte Werte"),
            help: &en_de(
                "What the row the object's keys select bounds, with its bounds, as \
                 `keyed-limit` measures it: one item for a quantity of the object, one per \
                 floor for a sill height, one per floor a side of a door may step onto for a \
                 threshold step; none where no row with a bound applies.",
                "Was die Zeile, die die Schlüssel des Objekts wählen, begrenzt, mit ihren \
                 Grenzen, wie `keyed-limit` es misst: ein Element für eine Größe des Objekts, \
                 eines je Boden für eine Brüstungshöhe, eines je Boden, auf den eine Türseite \
                 treten kann, für eine Schwellenstufe; keines, wo keine Zeile mit Grenze gilt.",
            ),
        },
        fields: &[
            field(
                "row",
                RATIO,
                &en_de("Row", "Zeile"),
                &en_de(
                    "The index of the row the object's keys select, from 0.",
                    "Der Index der Zeile, die die Schlüssel des Objekts wählen, ab 0.",
                ),
            ),
            field(
                "keys",
                MemberFieldKind::Text,
                &en_de("Keys", "Schlüssel"),
                &en_de(
                    "The keys as a message describes them: each property and its value.",
                    "Die Schlüssel, wie eine Meldung sie beschreibt: jede Eigenschaft und ihr \
                     Wert.",
                ),
            ),
            field(
                "value",
                RATIO,
                &en_de("Value", "Wert"),
                &en_de(
                    "The bounded value in coherent SI units, as displayed against the bounds \
                     for a door's values and steps; undecided where it cannot be measured.",
                    "Der begrenzte Wert in kohärenten SI-Einheiten, für Türwerte und Stufen wie \
                     gegen die Grenzen angezeigt; unentschieden, wo er nicht messbar ist.",
                ),
            ),
            field(
                "minimum",
                RATIO,
                &en_de("Minimum", "Minimum"),
                &en_de(
                    "The row's minimum; `null` where none.",
                    "Das Minimum der Zeile; `null`, wo keines.",
                ),
            ),
            field(
                "maximum",
                RATIO,
                &en_de("Maximum", "Maximum"),
                &en_de(
                    "The row's maximum; `null` where none.",
                    "Das Maximum der Zeile; `null`, wo keines.",
                ),
            ),
            field(
                "what",
                MemberFieldKind::Text,
                &en_de("What", "Was"),
                &en_de(
                    "How a message names the value and how it was read.",
                    "Wie eine Meldung den Wert nennt und wie er gelesen wurde.",
                ),
            ),
            field(
                "unit",
                MemberFieldKind::Text,
                &en_de("Unit", "Einheit"),
                &en_de(
                    "The value's unit after a space; empty for a plain number.",
                    "Die Einheit des Werts nach einem Leerzeichen; leer für eine reine Zahl.",
                ),
            ),
            field(
                "named",
                MemberFieldKind::Text,
                &en_de("Step", "Stufe"),
                &en_de(
                    "A threshold step as a message names it, its floor and measure included.",
                    "Eine Schwellenstufe, wie eine Meldung sie nennt, mit Boden und Maß.",
                ),
            ),
            field(
                "side",
                MemberFieldKind::Text,
                &en_de("Side", "Seite"),
                &en_de(
                    "The space a threshold step's side lies beside.",
                    "Der Raum, neben dem die Seite einer Schwellenstufe liegt.",
                ),
            ),
            field(
                "sure",
                TRUTH,
                &en_de("Sure floor", "Sicherer Boden"),
                &en_de(
                    "Whether the side surely steps onto this floor; it is one of several possible ones otherwise.",
                    "Ob die Seite sicher auf diesen Boden tritt; sonst ist er einer von mehreren möglichen.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "The objects a key was read on, and the floor or members the value was measured with.",
                    "Die Objekte, an denen ein Schlüssel gelesen wurde, und der Boden oder die Glieder, mit denen der Wert gemessen wurde.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "name_sequence",
            parameters: &[
                MeasuredParameter {
                    key: "member_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The members numbered: the objects a selector parameter of the rule picks.",
                        "Die nummerierten Mitglieder: die Objekte, die ein Selektorparameter der Regel wählt.",
                    ),
                },
                MeasuredParameter {
                    key: "name",
                    kind: MeasuredParameterKind::Property,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The property holding a member's number.",
                        "Die Eigenschaft, die die Nummer eines Mitglieds trägt.",
                    ),
                },
                MeasuredParameter {
                    key: "order",
                    kind: MeasuredParameterKind::Property,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The numeric property the members are ordered by.",
                        "Die numerische Eigenschaft, nach der die Mitglieder geordnet werden.",
                    ),
                },
                MeasuredParameter {
                    key: "first",
                    kind: MeasuredParameterKind::Number { minimum: f64::MIN },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The number the first member must state; 1 without it.",
                        "Die Nummer, die das erste Mitglied angeben muss; ohne Angabe 1.",
                    ),
                },
                MeasuredParameter {
                    key: "increment",
                    kind: MeasuredParameterKind::Number { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How much each number exceeds the one before; 1 without it.",
                        "Um wie viel jede Nummer die vorige übersteigt; ohne Angabe 1.",
                    ),
                },
                MeasuredParameter {
                    key: "order_fallback",
                    kind: MeasuredParameterKind::Choice {
                        options: &["placement_height"],
                    },
                    required: false,
                    default: None,
                    help: &en_de(
                        "With `placement_height`, a member stating no order value is ordered by the height of its placement.",
                        "Mit `placement_height` wird ein Mitglied ohne Ordnungswert nach der Höhe seiner Platzierung geordnet.",
                    ),
                },
                TRAVERSAL[0],
                TRAVERSAL[1],
                TRAVERSAL[2],
                TRAVERSAL[3],
                TRAVERSAL[4],
            ],
            dimension: None,
            services: &["relationship-selection", "object-frame"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "the member selection is undecided",
                "a member states no numeric order value",
            ],
            label: &en_de("Name sequence", "Namensfolge"),
            help: &en_de(
                "The members an object reaches, in order, each with its number and the number \
                 the sequence expects of it, as `name-sequence` reads them.",
                "Die Mitglieder, die ein Objekt erreicht, in Reihenfolge, jedes mit seiner \
                 Nummer und der Nummer, die die Folge von ihm erwartet, wie `name-sequence` sie \
                 liest.",
            ),
        },
        fields: &[
            field(
                "member",
                MemberFieldKind::Objects,
                &en_de("Member", "Mitglied"),
                &en_de("The member, in order.", "Das Mitglied, in der Reihenfolge."),
            ),
            field(
                "shown",
                MemberFieldKind::Text,
                &en_de("Stated", "Angegeben"),
                &en_de(
                    "What the member states, as findings show it.",
                    "Was das Mitglied angibt, wie Befunde es zeigen.",
                ),
            ),
            field(
                "set",
                TRUTH,
                &en_de("Set", "Gesetzt"),
                &en_de(
                    "Whether the member states a value.",
                    "Ob das Mitglied einen Wert angibt.",
                ),
            ),
            field(
                "whole",
                TRUTH,
                &en_de("Whole number", "Ganze Zahl"),
                &en_de(
                    "Whether the value is a whole number: optional sign and digits.",
                    "Ob der Wert eine ganze Zahl ist: optionales Vorzeichen und Ziffern.",
                ),
            ),
            field(
                "value",
                RATIO,
                &en_de("Number", "Nummer"),
                &en_de(
                    "The member's number; `null` where it states none.",
                    "Die Nummer des Mitglieds; `null`, wo es keine angibt.",
                ),
            ),
            field(
                "previous",
                RATIO,
                &en_de("Previous", "Vorige"),
                &en_de(
                    "The number of the member below it in the sequence; `null` for the first.",
                    "Die Nummer des Mitglieds darunter in der Folge; `null` für das erste.",
                ),
            ),
            field(
                "expected",
                RATIO,
                &en_de("Expected", "Erwartet"),
                &en_de(
                    "The number the sequence expects: the first, or the previous plus the increment.",
                    "Die Nummer, die die Folge erwartet: die erste, oder die vorige plus die Schrittweite.",
                ),
            ),
            field(
                "above",
                TRUTH,
                &en_de("Above", "Darüber"),
                &en_de(
                    "Whether the number lies above the previous one.",
                    "Ob die Nummer über der vorigen liegt.",
                ),
            ),
            field(
                "below",
                MemberFieldKind::Objects,
                &en_de("Below", "Darunter"),
                &en_de(
                    "The member below it in the sequence.",
                    "Das Mitglied darunter in der Folge.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "numbering",
            parameters: &[
                MeasuredParameter {
                    key: "property",
                    kind: MeasuredParameterKind::Property,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The property holding each object's number.",
                        "Die Eigenschaft, die die Nummer jedes Objekts trägt.",
                    ),
                },
                MeasuredParameter {
                    key: "pattern",
                    kind: MeasuredParameterKind::Pattern,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The XML Schema pattern over the whole value, its one group capturing the number.",
                        "Das XML-Schema-Muster über den ganzen Wert, dessen eine Gruppe die Nummer erfasst.",
                    ),
                },
                MeasuredParameter {
                    key: "prefix_length",
                    kind: MeasuredParameterKind::Number { minimum: 1.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How many leading digits the objects of a scope must share.",
                        "Wie viele führende Ziffern die Objekte eines Bereichs teilen müssen.",
                    ),
                },
                MeasuredParameter {
                    key: "gap_free",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether the numbers of a scope must leave no gap.",
                        "Ob die Nummern eines Bereichs keine Lücke lassen dürfen.",
                    ),
                },
                MeasuredParameter {
                    key: "across_sources",
                    kind: MeasuredParameterKind::Truth,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether the scopes span the sources.",
                        "Ob die Bereiche die Quellen übergreifen.",
                    ),
                },
                TRAVERSAL[0],
                TRAVERSAL[1],
                TRAVERSAL[2],
                TRAVERSAL[3],
                TRAVERSAL[4],
                MeasuredParameter {
                    key: "selection",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The objects numbered: the rule's own selection (`@selection`).",
                        "Die nummerierten Objekte: die eigene Auswahl der Regel (`@selection`).",
                    ),
                },
            ],
            dimension: None,
            services: &["property-resolution", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "the object's number cannot be read",
                "its number has too few digits for a prefix",
            ],
            label: &en_de("Numbering", "Nummerierung"),
            help: &en_de(
                "What `numbering-consistency` judges of an object's number among the numbers of \
                 its scope: why it cannot be read, its prefix's lead over the other prefixes and \
                 its step from the next lower number.",
                "Was `numbering-consistency` an der Nummer eines Objekts unter den Nummern seines \
                 Bereichs beurteilt: warum sie nicht gelesen werden kann, den Vorsprung ihres \
                 Präfixes vor den anderen und ihren Schritt von der nächstniedrigeren Nummer.",
            ),
        },
        fields: &[
            field(
                "unread",
                TRUTH,
                &en_de("Unread", "Ungelesen"),
                &en_de(
                    "True on the item stating why the object's number cannot be read.",
                    "Wahr am Element, das angibt, warum die Nummer des Objekts nicht gelesen werden kann.",
                ),
            ),
            field(
                "number",
                RATIO,
                &en_de("Number", "Nummer"),
                &en_de(
                    "Not evaluated, for why the number cannot be read.",
                    "Nicht ausgewertet, mit dem Grund, warum die Nummer nicht gelesen werden kann.",
                ),
            ),
            field(
                "prefixed",
                TRUTH,
                &en_de("Prefixed", "Präfix"),
                &en_de(
                    "True on the item judging the object's prefix.",
                    "Wahr am Element, das das Präfix des Objekts beurteilt.",
                ),
            ),
            field(
                "lead",
                RATIO,
                &en_de("Lead", "Vorsprung"),
                &en_de(
                    "How many more objects of the scope share its prefix than any other, less the objects not read; not evaluated for a number with too few digits.",
                    "Wie viele Objekte des Bereichs mehr sein Präfix teilen als irgendein anderes, abzüglich der nicht gelesenen; nicht ausgewertet für eine Nummer mit zu wenigen Ziffern.",
                ),
            ),
            field(
                "departs",
                MemberFieldKind::Text,
                &en_de("Departs", "Abweichung"),
                &en_de(
                    "Words naming how the object departs from its scope's prefix.",
                    "Worte, die nennen, wie das Objekt vom Präfix seines Bereichs abweicht.",
                ),
            ),
            field(
                "stepped",
                TRUTH,
                &en_de("Stepped", "Schritt"),
                &en_de(
                    "True on the item judging the object's step from the next lower number.",
                    "Wahr am Element, das den Schritt des Objekts von der nächstniedrigeren Nummer beurteilt.",
                ),
            ),
            field(
                "step",
                RATIO,
                &en_de("Step", "Schritt"),
                &en_de(
                    "How far its number lies above the next lower number of its scope.",
                    "Wie weit seine Nummer über der nächstniedrigeren Nummer seines Bereichs liegt.",
                ),
            ),
            field(
                "fillable",
                TRUTH,
                &en_de("Fillable", "Füllbar"),
                &en_de(
                    "Whether an object whose number could not be read may fill the gap below.",
                    "Ob ein Objekt, dessen Nummer nicht gelesen werden konnte, die Lücke darunter füllen kann.",
                ),
            ),
            field(
                "shown",
                MemberFieldKind::Text,
                &en_de("Stated", "Angegeben"),
                &en_de(
                    "What the object states, as findings show it.",
                    "Was das Objekt angibt, wie Befunde es zeigen.",
                ),
            ),
            field(
                "below",
                MemberFieldKind::Text,
                &en_de("Below", "Darunter"),
                &en_de(
                    "The next lower number of its scope.",
                    "Die nächstniedrigere Nummer seines Bereichs.",
                ),
            ),
            field(
                "missing",
                MemberFieldKind::Text,
                &en_de("Missing", "Fehlend"),
                &en_de(
                    "Words naming the numbers missing below.",
                    "Worte, die die fehlenden Nummern darunter nennen.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "The objects its prefix or step is judged against.",
                    "Die Objekte, gegen die sein Präfix oder Schritt beurteilt wird.",
                ),
            ),
            field(
                "reason",
                MemberFieldKind::Text,
                &en_de("Reason", "Grund"),
                &en_de(
                    "Why the item is undecided where not for incomplete evidence, as a report writes it (`missing_service`).",
                    "Warum das Element unentschieden ist, wo nicht wegen unvollständiger Belege, wie ein Bericht es schreibt (`missing_service`).",
                ),
            ),
        ],
    },
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
            subject: MeasuredSubject::Object,
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
            subject: MeasuredSubject::Object,
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
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "parking_bay",
            parameters: &[
                MeasuredParameter {
                    key: "min_width",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The least width of a bay, if any.",
                        "Die kleinste Breite eines Stellplatzes, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "max_width",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The greatest width of a bay, if any.",
                        "Die größte Breite eines Stellplatzes, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "min_length",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The least length of a bay, if any.",
                        "Die kleinste Länge eines Stellplatzes, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "max_length",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The greatest length of a bay, if any.",
                        "Die größte Länge eines Stellplatzes, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "min_height",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The least height of a bay, if any.",
                        "Die kleinste Höhe eines Stellplatzes, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "max_height",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The greatest height of a bay, if any.",
                        "Die größte Höhe eines Stellplatzes, falls angegeben.",
                    ),
                },
                MeasuredParameter {
                    key: "aisles",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The aisles a bay's orientation is read against.",
                        "Die Fahrgassen, gegen die die Ausrichtung eines Stellplatzes gelesen wird.",
                    ),
                },
                MeasuredParameter {
                    key: "aisle_reach",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far from the bay an aisle may lie.",
                        "Wie weit vom Stellplatz eine Fahrgasse liegen darf.",
                    ),
                },
                MeasuredParameter {
                    key: "orientation",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The orientation a bay must have to an aisle, as the rule states it.",
                        "Die Ausrichtung, die ein Stellplatz zu einer Fahrgasse haben muss, wie die Regel sie angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "angle_tolerance",
                    kind: MeasuredParameterKind::Angle { below: 45.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far from parallel or perpendicular axes may lie, in degrees.",
                        "Wie weit Achsen von parallel oder rechtwinklig abweichen dürfen, in Grad.",
                    ),
                },
                MeasuredParameter {
                    key: "obstacles",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The objects that may obstruct a bay.",
                        "Die Objekte, die einen Stellplatz behindern können.",
                    ),
                },
                MeasuredParameter {
                    key: "obstruction_reach",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far from the bay an obstacle may lie.",
                        "Wie weit vom Stellplatz ein Hindernis liegen darf.",
                    ),
                },
                MeasuredParameter {
                    key: "end_obstructions",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "How many ends may be obstructed, as the rule states it.",
                        "Wie viele Enden behindert sein dürfen, wie die Regel es angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "side_obstructions",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "How many sides may be obstructed, as the rule states it.",
                        "Wie viele Seiten behindert sein dürfen, wie die Regel es angibt.",
                    ),
                },
                MeasuredParameter {
                    key: "applies_when",
                    kind: MeasuredParameterKind::Pattern,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Whether orientation and obstructions are findings or select the bays the sizes apply to.",
                        "Ob Ausrichtung und Behinderungen Befunde sind oder die Stellplätze wählen, für die die Größen gelten.",
                    ),
                },
                MeasuredParameter {
                    key: "orientations",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The orientation states the sizes apply to, as the rule lists them.",
                        "Die Ausrichtungszustände, für die die Größen gelten, wie die Regel sie aufzählt.",
                    ),
                },
                MeasuredParameter {
                    key: "end_states",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The obstructed ends the sizes apply to, as the rule lists them.",
                        "Die behinderten Enden, für die die Größen gelten, wie die Regel sie aufzählt.",
                    ),
                },
                MeasuredParameter {
                    key: "side_states",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The obstructed sides the sizes apply to, as the rule lists them.",
                        "Die behinderten Seiten, für die die Größen gelten, wie die Regel sie aufzählt.",
                    ),
                },
                MeasuredParameter {
                    key: "side_zone_length",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The central stretch of a side an obstruction must overlap.",
                        "Der mittlere Abschnitt einer Seite, den eine Behinderung überlappen muss.",
                    ),
                },
                MeasuredParameter {
                    key: "neighbour_reach",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far from the bay a neighbouring bay may lie.",
                        "Wie weit vom Stellplatz ein benachbarter Stellplatz liegen darf.",
                    ),
                },
                MeasuredParameter {
                    key: "selection",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The rule's bays (`@selection`), among which neighbours are read.",
                        "Die Stellplätze der Regel (`@selection`), unter denen Nachbarn gelesen werden.",
                    ),
                },
            ],
            dimension: None,
            services: &["plan-span", "proximity", "vertical-extent"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "a service is not registered",
                "the bay's extent cannot be read",
            ],
            label: &en_de("Parking bay", "Stellplatz"),
            help: &en_de(
                "What `parking-bay` judges of a bay: its sizes against their bounds, the \
                 obstacles counted within it and at its ends and sides against what is \
                 allowed, and its orientation to an aisle.",
                "Was `parking-bay` an einem Stellplatz beurteilt: seine Größen gegen ihre \
                 Grenzen, die in ihm und an seinen Enden und Seiten gezählten Hindernisse gegen \
                 das Erlaubte, und seine Ausrichtung zu einer Fahrgasse.",
            ),
        },
        fields: &[
            field(
                "judged",
                TRUTH,
                &en_de("Search", "Suche"),
                &en_de(
                    "True on an item of a search's own answer.",
                    "Wahr am Element der eigenen Antwort einer Suche.",
                ),
            ),
            field(
                "found",
                TRUTH,
                &en_de("Found", "Gefunden"),
                &en_de(
                    "Whether the search found the bay wanting; refused for why it is undecided.",
                    "Ob die Suche den Stellplatz mangelhaft fand; verweigert mit dem Grund, warum es offen ist.",
                ),
            ),
            field(
                "message",
                MemberFieldKind::Text,
                &en_de("Words", "Worte"),
                &en_de(
                    "The search's finding, worded.",
                    "Der Befund der Suche, in Worten.",
                ),
            ),
            field(
                "counting",
                TRUTH,
                &en_de("Count", "Zählung"),
                &en_de(
                    "True on an item of obstacles counted.",
                    "Wahr am Element gezählter Hindernisse.",
                ),
            ),
            field(
                "count",
                RATIO,
                &en_de("Counted", "Gezählt"),
                &en_de(
                    "From the obstacles or edges surely counted to every one that may be.",
                    "Von den sicher gezählten Hindernissen oder Kanten bis zu jedem, das es sein kann.",
                ),
            ),
            field(
                "allowed",
                RATIO,
                &en_de("Allowed", "Erlaubt"),
                &en_de("How many are allowed.", "Wie viele erlaubt sind."),
            ),
            field(
                "found_words",
                MemberFieldKind::Text,
                &en_de("Finding", "Befund"),
                &en_de("Words of a finding.", "Worte eines Befunds."),
            ),
            field(
                "open_words",
                MemberFieldKind::Text,
                &en_de("Doubt", "Zweifel"),
                &en_de("Words of a doubt.", "Worte eines Zweifels."),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "The objects a finding relates.",
                    "Die Objekte, auf die ein Befund verweist.",
                ),
            ),
            field(
                "sized",
                TRUTH,
                &en_de("Size", "Größe"),
                &en_de("True on an item of a size.", "Wahr am Element einer Größe."),
            ),
            field(
                "size",
                LENGTH,
                &en_de("Measured", "Gemessen"),
                &en_de(
                    "The size; refused for why it cannot be measured.",
                    "Die Größe; verweigert mit dem Grund, warum sie nicht gemessen werden kann.",
                ),
            ),
            field(
                "measured",
                MemberFieldKind::Text,
                &en_de("Words", "Worte"),
                &en_de("The size, worded.", "Die Größe, in Worten."),
            ),
            field(
                "low",
                LENGTH,
                &en_de("Least", "Mindestens"),
                &en_de(
                    "The least size allowed, if any.",
                    "Die kleinste erlaubte Größe, falls angegeben.",
                ),
            ),
            field(
                "high",
                LENGTH,
                &en_de("Most", "Höchstens"),
                &en_de(
                    "The greatest size allowed, if any.",
                    "Die größte erlaubte Größe, falls angegeben.",
                ),
            ),
            field(
                "suffix",
                MemberFieldKind::Text,
                &en_de("States", "Zustände"),
                &en_de(
                    "The states a filtered bay is in, after a finding.",
                    "Die Zustände eines gefilterten Stellplatzes, nach einem Befund.",
                ),
            ),
            field(
                "doubtful",
                TRUTH,
                &en_de("Doubtful", "Zweifelhaft"),
                &en_de(
                    "Whether the filters may leave the bay out.",
                    "Ob die Filter den Stellplatz auslassen können.",
                ),
            ),
            field(
                "applies_why",
                MemberFieldKind::Text,
                &en_de("Why", "Warum"),
                &en_de(
                    "Why the filters may leave the bay out.",
                    "Warum die Filter den Stellplatz auslassen können.",
                ),
            ),
            field(
                "later",
                MemberFieldKind::Text,
                &en_de("Then", "Danach"),
                &en_de(
                    "Why the filters may leave the bay out, after `; `, where they may.",
                    "Warum die Filter den Stellplatz auslassen können, nach `; `, wo sie es können.",
                ),
            ),
            field(
                "reason",
                MemberFieldKind::Text,
                &en_de("Reason", "Grund"),
                &en_de(
                    "Why the item is undecided where not for incomplete evidence, as a report writes it (`missing_service`).",
                    "Warum das Element unentschieden ist, wo nicht wegen unvollständiger Belege, wie ein Bericht es schreibt (`missing_service`).",
                ),
            ),
        ],
    },
    walking::RAIL_CONTINUITY,
    walking::RAIL_EXTENSIONS,
    walking::RAIL_GAPS,
    walking::RAIL_HEIGHTS,
    walking::RAIL_OBSTRUCTIONS,
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "recesses",
            parameters: &[MeasuredParameter {
                key: "requirements",
                kind: MeasuredParameterKind::Table,
                required: false,
                default: None,
                help: &en_de(
                    "Rows of an optional depth range (`minimum_depth_metres` exclusive, \
                     `maximum_depth_metres` inclusive) and the width they require \
                     (`minimum_width_metres`, `minimum_width_per_depth` times the depth, \
                     the larger): the first row whose range holds a recess's depth is its \
                     row.",
                    "Zeilen aus einem optionalen Tiefenbereich (`minimum_depth_metres` \
                     ausschließlich, `maximum_depth_metres` einschließlich) und der \
                     verlangten Breite (`minimum_width_metres`, `minimum_width_per_depth` \
                     mal die Tiefe, die größere): die erste Zeile, deren Bereich die Tiefe \
                     eines Rücksprungs hält, ist seine Zeile.",
                ),
            }],
            dimension: None,
            services: &["plan-span"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "the plan-span service does not measure the recesses",
                "a recess's depth straddles a row's bound: its row and required width are \
                 undecided",
            ],
            label: &en_de("Recesses", "Rücksprünge"),
            help: &en_de(
                "The pockets between a footprint's outer boundary and its convex hull.",
                "Die Taschen zwischen dem Außenrand eines Grundrisses und seiner konvexen \
                 Hülle.",
            ),
        },
        fields: &[
            field(
                "place",
                MemberFieldKind::Text,
                &en_de("Place", "Lage"),
                &en_de(
                    "Where the recess's mouth lies: `recess at (x, y)-(x, y)`, in metres.",
                    "Wo die Öffnung des Rücksprungs liegt: `recess at (x, y)-(x, y)`, in \
                     Metern.",
                ),
            ),
            field(
                "row",
                RATIO,
                &en_de("Row", "Zeile"),
                &en_de(
                    "The index of the first `requirements` row whose depth range holds the \
                     recess, from 0; `null` where none does or no rows are given.",
                    "Der Index der ersten Zeile von `requirements`, deren Tiefenbereich den \
                     Rücksprung hält, ab 0; `null`, wo keine es tut oder keine Zeilen \
                     angegeben sind.",
                ),
            ),
            field(
                "required",
                LENGTH,
                &en_de("Required width", "Verlangte Breite"),
                &en_de(
                    "The width the recess's row requires for its depth; `null` without a \
                     row.",
                    "Die Breite, die die Zeile des Rücksprungs für seine Tiefe verlangt; \
                     `null` ohne Zeile.",
                ),
            ),
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
    searches::ROUTE_VERDICTS,
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
            subject: MeasuredSubject::Object,
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
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "sight_view",
            parameters: &[
                MeasuredParameter {
                    key: "targets",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The targets looked for: source kinds or the objects a selector \
                         parameter of the rule picks (`@name`), those it leaves undecided \
                         possible targets.",
                        "Die gesuchten Ziele: Quellarten oder die Objekte, die ein \
                         Selektorparameter der Regel wählt (`@name`), die unentschiedenen \
                         mögliche Ziele.",
                    ),
                },
                MeasuredParameter {
                    key: "blockers",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The objects that may hide a target, those the selection leaves \
                         undecided possible blockers.",
                        "Die Objekte, die ein Ziel verdecken können, die von der Auswahl \
                         unentschiedenen mögliche Verdecker.",
                    ),
                },
                MeasuredParameter {
                    key: "eye_height",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: true,
                    default: None,
                    help: &en_de(
                        "How far above the component's base the eye stands, over the centre \
                         of its footprint.",
                        "Wie weit über der Unterkante des Bauteils das Auge steht, über dem \
                         Mittelpunkt seines Grundrisses.",
                    ),
                },
                MeasuredParameter {
                    key: "radius",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: true,
                    default: None,
                    help: &en_de(
                        "How far from the eye a target is looked for.",
                        "Wie weit vom Auge ein Ziel gesucht wird.",
                    ),
                },
            ],
            dimension: None,
            services: &["line-of-sight", "plan-span", "vertical-extent"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "a service is not registered",
                "the component's centre or base is not known exactly",
            ],
            label: &en_de("View from the eye", "Sicht vom Auge"),
            help: &en_de(
                "One item: the targets within the radius in view from an eye above the \
                 component, as `component-visibility` sees them: surely in view, possibly in \
                 view, or hidden.",
                "Ein Element: die Ziele innerhalb des Radius, die von einem Auge über dem \
                 Bauteil zu sehen sind, wie `component-visibility` sie sieht: sicher sichtbar, \
                 möglicherweise sichtbar oder verdeckt.",
            ),
        },
        fields: &[
            field(
                "visible",
                RATIO,
                &en_de("Targets in view", "Sichtbare Ziele"),
                &en_de(
                    "From the targets surely in view to every target that may be.",
                    "Von den sicher sichtbaren Zielen bis zu jedem, das es sein kann.",
                ),
            ),
            field(
                "sure",
                RATIO,
                &en_de("Targets surely in view", "Sicher sichtbare Ziele"),
                &en_de(
                    "The targets surely in view.",
                    "Die sicher sichtbaren Ziele.",
                ),
            ),
            field(
                "hidden",
                RATIO,
                &en_de("Hidden targets", "Verdeckte Ziele"),
                &en_de(
                    "The targets within the radius surely hidden.",
                    "Die Ziele innerhalb des Radius, die sicher verdeckt sind.",
                ),
            ),
            field(
                "seen",
                MemberFieldKind::Objects,
                &en_de("In view", "Sichtbar"),
                &en_de(
                    "The targets surely in view.",
                    "Die sicher sichtbaren Ziele.",
                ),
            ),
            field(
                "found",
                MemberFieldKind::Objects,
                &en_de("Decided", "Entschieden"),
                &en_de(
                    "The targets surely in view, then those surely hidden.",
                    "Die sicher sichtbaren Ziele, dann die sicher verdeckten.",
                ),
            ),
            field(
                "within",
                MemberFieldKind::Text,
                &en_de("Eye", "Auge"),
                &en_de(
                    "Words naming the eye and the radius.",
                    "Worte, die Auge und Radius nennen.",
                ),
            ),
            field(
                "undecided",
                MemberFieldKind::Text,
                &en_de("Undecided targets", "Unentschiedene Ziele"),
                &en_de(
                    "How many targets are undecided, and why, for the first three.",
                    "Wie viele Ziele unentschieden sind, und warum, für die ersten drei.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "space_connections",
            parameters: &[
                MeasuredParameter {
                    key: "connections",
                    kind: MeasuredParameterKind::Table,
                    required: true,
                    default: None,
                    help: &en_de(
                        "Rows of a `from` selector (the spaces a row applies to), an \
                         optional `to` selector, `access` and `exit` (`allowed`, `required` \
                         or `forbidden`), an `access_type` (`any`, `doors` or `openings`) \
                         and an optional `label`, as `space-connection` reads them.",
                        "Zeilen aus einem Selektor `from` (die Räume, für die eine Zeile \
                         gilt), einem optionalen Selektor `to`, `access` und `exit` \
                         (`allowed`, `required` oder `forbidden`), einem `access_type` \
                         (`any`, `doors` oder `openings`) und einem optionalen `label`, wie \
                         `space-connection` sie liest.",
                    ),
                },
                MeasuredParameter {
                    key: "access_path",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from each door or opening to the spaces it \
                         connects.",
                        "Die Beziehungsschritte von jeder Tür oder Öffnung zu den Räumen, \
                         die sie verbindet.",
                    ),
                },
                MeasuredParameter {
                    key: "door_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The doors: source kinds, `,`-separated, or `@` a selector parameter \
                         of the rule, whose undecided objects may be doors.",
                        "Die Türen: Quellarten, durch `,` getrennt, oder mit `@` ein \
                         Selektorparameter der Regel, dessen unentschiedene Objekte Türen \
                         sein können.",
                    ),
                },
                MeasuredParameter {
                    key: "opening_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The openings, as `door_selector` names the doors.",
                        "Die Öffnungen, wie `door_selector` die Türen nennt.",
                    ),
                },
                MeasuredParameter {
                    key: "space_selector",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The spaces a door or opening may connect; every object without it.",
                        "Die Räume, die eine Tür oder Öffnung verbinden kann; ohne Angabe \
                         jedes Objekt.",
                    ),
                },
            ],
            dimension: None,
            services: &["relationship-selection"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &["whether a row's `from` picks the space is undecided"],
            label: &en_de("Space connections", "Raumverbindungen"),
            help: &en_de(
                "Each requirement of each row of `connections` whose `from` picks the \
                 space, its `access` and then its `exit`: whether the space is surely \
                 linked as the row asks, surely not, or undecided.",
                "Jede Anforderung jeder Zeile von `connections`, deren `from` den Raum \
                 trifft, erst ihr `access`, dann ihr `exit`: ob der Raum sicher wie \
                 verlangt verbunden ist, sicher nicht, oder unentschieden.",
            ),
        },
        fields: &[
            field(
                "row",
                MemberFieldKind::Text,
                &en_de("Row", "Zeile"),
                &en_de(
                    "The row, as messages name it: `row 0 (label)`.",
                    "Die Zeile, wie Meldungen sie nennen: `row 0 (label)`.",
                ),
            ),
            field(
                "access",
                TRUTH,
                &en_de("Access", "Zugang"),
                &en_de(
                    "Whether the item is the row's `access` requirement; its `exit` \
                     otherwise.",
                    "Ob das Element die Anforderung `access` der Zeile ist; sonst ihr \
                     `exit`.",
                ),
            ),
            field(
                "required",
                TRUTH,
                &en_de("Required", "Verlangt"),
                &en_de(
                    "Whether the row requires the link; it forbids it otherwise.",
                    "Ob die Zeile die Verbindung verlangt; sonst verbietet sie sie.",
                ),
            ),
            field(
                "kind",
                MemberFieldKind::Text,
                &en_de("Element type", "Elementart"),
                &en_de(
                    "The elements the row counts: `door`, `opening` or `door or opening`.",
                    "Die Elemente, die die Zeile zählt: `door`, `opening` oder \
                     `door or opening`.",
                ),
            ),
            field(
                "via",
                MemberFieldKind::Text,
                &en_de("Relationship", "Beziehung"),
                &en_de(
                    "The relationship the access path follows.",
                    "Die Beziehung, der der Zugangspfad folgt.",
                ),
            ),
            field(
                "links",
                MemberFieldKind::Text,
                &en_de("Links", "Verbindungen"),
                &en_de(
                    "The sure links found: each space and the element it is reached \
                     through, or the element opening to the outside; empty where none.",
                    "Die sicheren Verbindungen: je Raum und das Element, durch das er \
                     erreicht wird, oder das Element ins Freie; leer, wo keine.",
                ),
            ),
            field(
                "linked",
                TRUTH,
                &en_de("Linked", "Verbunden"),
                &en_de(
                    "Whether the space is surely linked as the row asks; undecided where an \
                     element or a space the row may pick could change it.",
                    "Ob der Raum sicher wie verlangt verbunden ist; unentschieden, wo ein \
                     Element oder ein Raum, den die Zeile treffen kann, es ändern könnte.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "The linked spaces and elements where linked; otherwise the elements \
                     reaching the space.",
                    "Die verbundenen Räume und Elemente, wo verbunden; sonst die Elemente, \
                     die den Raum erreichen.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "space_overlaps",
            parameters: &[SPACE_SELECTED, SPACE_TOLERANCE],
            dimension: None,
            services: SPACE,
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[SPACE_UNMEASURED, SPACE_UNDECIDED],
            label: &en_de("Intersecting bodies", "Durchdringende Körper"),
            help: &en_de(
                "Each body a space contains, is contained by or intersects (a partial \
                 overlap thicker than `tolerance`), as `space-validation` judges them.",
                "Jeder Körper, den ein Raum enthält, der ihn enthält oder ihn durchdringt \
                 (eine teilweise Überlappung dicker als `tolerance`), wie \
                 `space-validation` sie beurteilt.",
            ),
        },
        fields: &[
            field(
                "inside",
                TRUTH,
                &en_de("Inside", "Innerhalb"),
                &en_de(
                    "Whether the space lies inside the body.",
                    "Ob der Raum im Körper liegt.",
                ),
            ),
            field(
                "contains",
                TRUTH,
                &en_de("Contains", "Enthält"),
                &en_de(
                    "Whether the space contains the body.",
                    "Ob der Raum den Körper enthält.",
                ),
            ),
            field(
                "space",
                TRUTH,
                &en_de("Another space", "Anderer Raum"),
                &en_de(
                    "Whether the body partly overlapping it is another space.",
                    "Ob der teilweise überlappende Körper ein anderer Raum ist.",
                ),
            ),
            field(
                "partial",
                TRUTH,
                &en_de("Partial", "Teilweise"),
                &en_de(
                    "Whether the body partly overlaps it.",
                    "Ob der Körper ihn teilweise überlappt.",
                ),
            ),
            field(
                "area",
                AREA,
                &en_de("Overlap area", "Überlappungsfläche"),
                &en_de(
                    "The area they overlap over.",
                    "Die Fläche, über die sie sich überlappen.",
                ),
            ),
            field(
                "other",
                MemberFieldKind::Objects,
                &en_de("Body", "Körper"),
                &en_de("The body it overlaps.", "Der überlappte Körper."),
            ),
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
            subject: MeasuredSubject::Object,
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
                MeasuredParameter {
                    key: "towards",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Spaces the door is judged to swing towards: source kinds, \
                         `,`-separated, or `@` a selector parameter of the rule. With it or \
                         `not_towards`, only the spaces either may pick are listed and \
                         probed, undecided ones too.",
                        "Räume, zu denen die Tür aufschlagen soll: Quellarten, durch `,` \
                         getrennt, oder mit `@` ein Selektorparameter der Regel. Mit ihr oder \
                         `not_towards` werden nur die Räume aufgeführt und geprüft, die eine \
                         der beiden treffen kann, unentschiedene eingeschlossen.",
                    ),
                },
                MeasuredParameter {
                    key: "not_towards",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Spaces the door is judged not to swing towards, as `towards`.",
                        "Räume, zu denen die Tür nicht aufschlagen soll, wie `towards`.",
                    ),
                },
            ],
            dimension: None,
            services: &["object-frame", "free-space", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
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
                "space",
                MemberFieldKind::Objects,
                &en_de("Space", "Raum"),
                &en_de("The space reached.", "Der erreichte Raum."),
            ),
            field(
                "towards",
                TRUTH,
                &en_de("Towards", "Hin"),
                &en_de(
                    "Whether `towards` surely picks the space; undecided where its \
                     selection cannot tell, true without it.",
                    "Ob `towards` den Raum sicher auswählt; unentschieden, wo die Auswahl es \
                     nicht entscheidet, wahr ohne sie.",
                ),
            ),
            field(
                "not_towards",
                TRUTH,
                &en_de("Not towards", "Nicht hin"),
                &en_de(
                    "Whether `not_towards` surely picks the space, as `towards`.",
                    "Ob `not_towards` den Raum sicher auswählt, wie `towards`.",
                ),
            ),
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
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "unallocated_regions",
            parameters: &[],
            dimension: None,
            services: SPACE,
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Project,
            not_evaluated: &[SPACE_UNMEASURED],
            label: &en_de("Unallocated floor", "Nicht zugeordnete Fläche"),
            help: &en_de(
                "Each connected region of storey floor that belongs to no space, as \
                 `space-validation` judges them, of the project.",
                "Jede zusammenhängende Geschossfläche, die keinem Raum gehört, wie \
                 `space-validation` sie beurteilt, im Projekt.",
            ),
        },
        fields: &[
            field(
                "storey",
                MemberFieldKind::Objects,
                &en_de("Storey", "Geschoss"),
                &en_de("The storey it lies on.", "Das Geschoss, auf dem sie liegt."),
            ),
            field(
                "area",
                AREA,
                &en_de("Area", "Fläche"),
                &en_de("The region's area.", "Die Fläche des Bereichs."),
            ),
            field(
                "elements",
                MemberFieldKind::Objects,
                &en_de("Bodies around", "Umgebende Körper"),
                &en_de(
                    "The bodies around the region.",
                    "Die Körper um den Bereich.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "unallocated_storeys",
            parameters: &[],
            dimension: None,
            services: SPACE,
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Project,
            not_evaluated: &[SPACE_UNMEASURED],
            label: &en_de(
                "Unallocated storey floor",
                "Nicht zugeordnete Geschossfläche",
            ),
            help: &en_de(
                "Each storey with floor belonging to no space: its regions' summed area \
                 and share of its gross floor area, as `space-validation` judges them.",
                "Jedes Geschoss mit Fläche, die keinem Raum gehört: die summierte Fläche \
                 seiner Bereiche und ihr Anteil an seiner Bruttogeschossfläche, wie \
                 `space-validation` sie beurteilt.",
            ),
        },
        fields: &[
            field(
                "storey",
                MemberFieldKind::Objects,
                &en_de("Storey", "Geschoss"),
                &en_de("The storey.", "Das Geschoss."),
            ),
            field(
                "share",
                RATIO,
                &en_de("Unallocated share", "Nicht zugeordneter Anteil"),
                &en_de(
                    "The regions' summed area over the gross floor area; `null` where the \
                     service states none.",
                    "Die summierte Fläche der Bereiche über der Bruttogeschossfläche; `null`, \
                     wo der Dienst keine angibt.",
                ),
            ),
            field(
                "area",
                AREA,
                &en_de("Unallocated area", "Nicht zugeordnete Fläche"),
                &en_de(
                    "The regions' summed area, its lower end.",
                    "Die summierte Fläche der Bereiche, ihr unteres Ende.",
                ),
            ),
            field(
                "gross",
                AREA,
                &en_de("Gross floor area", "Bruttogeschossfläche"),
                &en_de(
                    "The storey's gross floor area.",
                    "Die Bruttogeschossfläche des Geschosses.",
                ),
            ),
            field(
                "elements",
                MemberFieldKind::Objects,
                &en_de("Bodies around", "Umgebende Körper"),
                &en_de(
                    "The bodies around its regions.",
                    "Die Körper um seine Bereiche.",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "wall_spacing",
            parameters: &[
                MeasuredParameter {
                    key: "members",
                    kind: MeasuredParameterKind::Objects,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The walls or beams paired: the objects a selector parameter of the rule picks, those it leaves undecided possible members.",
                        "Die gepaarten Wände oder Träger: die Objekte, die ein Selektorparameter der Regel wählt, die unentschiedenen mögliche Mitglieder.",
                    ),
                },
                MeasuredParameter {
                    key: "member_path",
                    kind: MeasuredParameterKind::Path,
                    required: true,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the storey to its members.",
                        "Die Beziehungsschritte vom Geschoss zu seinen Mitgliedern.",
                    ),
                },
                MeasuredParameter {
                    key: "angle_tolerance",
                    kind: MeasuredParameterKind::Angle { below: 45.0 },
                    required: true,
                    default: None,
                    help: &en_de(
                        "How far from parallel two members' long axes may lie, in degrees.",
                        "Wie weit die Längsachsen zweier Mitglieder von parallel abweichen dürfen, in Grad.",
                    ),
                },
                MeasuredParameter {
                    key: "minimum",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far apart parallel members must stand at least.",
                        "Wie weit parallele Mitglieder mindestens auseinanderstehen müssen.",
                    ),
                },
                MeasuredParameter {
                    key: "maximum",
                    kind: MeasuredParameterKind::Length { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "How far apart the members of a pair bounding a band stand at most.",
                        "Wie weit die Mitglieder eines ein Band begrenzenden Paars höchstens auseinanderstehen.",
                    ),
                },
                MeasuredParameter {
                    key: "footprints",
                    kind: MeasuredParameterKind::Objects,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The objects whose footprints the bands must cover.",
                        "Die Objekte, deren Grundrisse die Bänder überdecken müssen.",
                    ),
                },
                MeasuredParameter {
                    key: "footprint_path",
                    kind: MeasuredParameterKind::Path,
                    required: false,
                    default: None,
                    help: &en_de(
                        "The relationship steps from the storey to its footprint objects.",
                        "Die Beziehungsschritte vom Geschoss zu seinen Grundrissobjekten.",
                    ),
                },
                MeasuredParameter {
                    key: "uncovered_above",
                    kind: MeasuredParameterKind::Area { minimum: 0.0 },
                    required: false,
                    default: None,
                    help: &en_de(
                        "The area of a footprint outside every band allowed at most, in square metres.",
                        "Die höchstens erlaubte Fläche eines Grundrisses außerhalb aller Bänder, in Quadratmetern.",
                    ),
                },
            ],
            dimension: None,
            services: &[
                "plan-span",
                "proximity",
                "vertical-extent",
                "plan-area",
                "relationship-selection",
            ],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "a service is not registered",
                "the path to the members cannot be read",
            ],
            label: &en_de("Wall spacing", "Wandabstand"),
            help: &en_de(
                "What `wall-spacing` judges of a storey: each pair of members surely parallel and \
                 facing with its distance, why some pair may stand closer than the minimum, and \
                 the area of each footprint outside every band between pairs at most the maximum \
                 apart.",
                "Was `wall-spacing` an einem Geschoss beurteilt: jedes sicher parallele und \
                 gegenüberstehende Paar von Mitgliedern mit seinem Abstand, warum ein Paar näher \
                 als das Minimum stehen kann, und die Fläche jedes Grundrisses außerhalb aller \
                 Bänder zwischen Paaren höchstens das Maximum auseinander.",
            ),
        },
        fields: &[
            field(
                "close",
                TRUTH,
                &en_de("Pair", "Paar"),
                &en_de(
                    "True on an item of a pair surely parallel, facing and selected.",
                    "Wahr am Element eines sicher parallelen, gegenüberstehenden und gewählten Paars.",
                ),
            ),
            field(
                "distance",
                LENGTH,
                &en_de("Distance", "Abstand"),
                &en_de(
                    "The pair's plan distance.",
                    "Der Grundrissabstand des Paars.",
                ),
            ),
            field(
                "apart",
                MemberFieldKind::Text,
                &en_de("Apart", "Abstand"),
                &en_de(
                    "The pair's plan distance as findings show it.",
                    "Der Grundrissabstand des Paars, wie Befunde ihn zeigen.",
                ),
            ),
            field(
                "pair",
                MemberFieldKind::Objects,
                &en_de("Members", "Mitglieder"),
                &en_de(
                    "The pair's two members.",
                    "Die beiden Mitglieder des Paars.",
                ),
            ),
            field(
                "spacing",
                LENGTH,
                &en_de("Spacing", "Abstand"),
                &en_de(
                    "Not evaluated, for why some pair may stand closer than the minimum.",
                    "Nicht ausgewertet, mit dem Grund, warum ein Paar näher als das Minimum stehen kann.",
                ),
            ),
            field(
                "cover",
                TRUTH,
                &en_de("Cover", "Überdeckung"),
                &en_de(
                    "True on an item of a footprint the bands must cover.",
                    "Wahr am Element eines Grundrisses, den die Bänder überdecken müssen.",
                ),
            ),
            field(
                "uncovered",
                RATIO,
                &en_de("Uncovered", "Unüberdeckt"),
                &en_de(
                    "The footprint's area outside every band, in square metres; refused for why it cannot be measured.",
                    "Die Fläche des Grundrisses außerhalb aller Bänder, in Quadratmetern; verweigert mit dem Grund, warum sie nicht gemessen werden kann.",
                ),
            ),
            field(
                "what",
                MemberFieldKind::Text,
                &en_de("Words", "Worte"),
                &en_de(
                    "Words naming the uncovered area.",
                    "Worte, die die unüberdeckte Fläche nennen.",
                ),
            ),
            field(
                "unknown",
                MemberFieldKind::Text,
                &en_de("Unknown", "Unbekannt"),
                &en_de(
                    "Why the bands may cover more, each after `; `.",
                    "Warum die Bänder mehr überdecken können, jeweils nach `; `.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "The members bounding a sure band, and the footprint.",
                    "Die Mitglieder, die ein sicheres Band begrenzen, und der Grundriss.",
                ),
            ),
            field(
                "reason",
                MemberFieldKind::Text,
                &en_de("Reason", "Grund"),
                &en_de(
                    "Why the item is undecided where not for incomplete evidence, as a report writes it (`missing_service`).",
                    "Warum das Element unentschieden ist, wo nicht wegen unvollständiger Belege, wie ein Bericht es schreibt (`missing_service`).",
                ),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "well_gaps",
            parameters: &[WELL_MEMBERS],
            dimension: None,
            services: &["vertical-extent", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "the path reaches no space",
                "a stacked space's vertical extent cannot be measured",
            ],
            label: &en_de("Gaps in the well", "Lücken im Schacht"),
            help: &en_de(
                "Each pair of consecutive stacked spaces, ordered by their bottoms, with the \
                 vertical gap from the lower one's top to the upper one's bottom, as \
                 `light-well` judges contiguity.",
                "Jedes Paar aufeinanderfolgender gestapelter Räume, nach ihren Böden \
                 geordnet, mit der senkrechten Lücke von der Oberkante des unteren zum Boden \
                 des oberen, wie `light-well` die Durchgängigkeit prüft.",
            ),
        },
        fields: &[
            field(
                "gap",
                LENGTH,
                &en_de("Gap", "Lücke"),
                &en_de(
                    "From the lower space's top to the upper space's bottom, none below zero.",
                    "Von der Oberkante des unteren bis zum Boden des oberen Raums, nie unter \
                     null.",
                ),
            ),
            field(
                "below",
                MemberFieldKind::Objects,
                &en_de("Space below", "Raum darunter"),
                &en_de("The lower space of the pair.", "Der untere Raum des Paars."),
            ),
            field(
                "above",
                MemberFieldKind::Objects,
                &en_de("Space above", "Raum darüber"),
                &en_de("The upper space of the pair.", "Der obere Raum des Paars."),
            ),
            field(
                "members",
                MemberFieldKind::Objects,
                &en_de("Stacked spaces", "Gestapelte Räume"),
                &en_de("Every space of the well.", "Jeder Raum des Schachts."),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "well_requirements",
            parameters: &[
                WELL_MEMBERS,
                MeasuredParameter {
                    key: "requirements",
                    kind: MeasuredParameterKind::Table,
                    required: false,
                    default: None,
                    help: &en_de(
                        "Rows of an optional `maximum_height_metres` and the section's \
                         `minimum_area_square_metres` and `minimum_width_metres`: the first \
                         row whose maximum the well's height does not exceed is the well's \
                         row.",
                        "Zeilen aus einer optionalen `maximum_height_metres` und \
                         `minimum_area_square_metres` und `minimum_width_metres` des \
                         Querschnitts: die erste Zeile, deren Höchstwert die Höhe des \
                         Schachts nicht übersteigt, ist die Zeile des Schachts.",
                    ),
                },
            ],
            dimension: None,
            services: &["vertical-extent", "plan-span", "relationship-selection"],
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "the path reaches no space",
                "the shared plan section cannot be measured",
                "the well's height straddles a row's maximum: its row is undecided",
            ],
            label: &en_de("Well section by height", "Schachtquerschnitt nach Höhe"),
            help: &en_de(
                "One item: the plan section the stacked spaces share, the well's height \
                 from its lowest bottom to its highest top, and the row of `requirements` \
                 its height selects, as `light-well` judges them.",
                "Ein Element: der Grundrissquerschnitt, den die gestapelten Räume teilen, \
                 die Höhe des Schachts vom tiefsten Boden bis zur höchsten Oberkante und \
                 die Zeile von `requirements`, die seine Höhe wählt, wie `light-well` sie \
                 prüft.",
            ),
        },
        fields: &[
            field(
                "count",
                RATIO,
                &en_de("Stacked spaces", "Gestapelte Räume"),
                &en_de(
                    "How many spaces the well stacks.",
                    "Wie viele Räume der Schacht stapelt.",
                ),
            ),
            field(
                "area",
                AREA,
                &en_de("Section area", "Querschnittsfläche"),
                &en_de(
                    "The shared section's area; `null` where the spaces surely share none.",
                    "Die Fläche des geteilten Querschnitts; `null`, wo die Räume sicher \
                     keinen teilen.",
                ),
            ),
            field(
                "width",
                LENGTH,
                &en_de("Section width", "Querschnittsbreite"),
                &en_de(
                    "The short side of the section's least-area rectangle; `null` without \
                     one.",
                    "Die kurze Seite des kleinsten umschließenden Rechtecks des \
                     Querschnitts; `null` ohne eines.",
                ),
            ),
            field(
                "height",
                LENGTH,
                &en_de("Well height", "Schachthöhe"),
                &en_de(
                    "From the lowest bottom to the highest top of the stacked spaces.",
                    "Vom tiefsten Boden bis zur höchsten Oberkante der gestapelten Räume.",
                ),
            ),
            field(
                "row",
                RATIO,
                &en_de("Row", "Zeile"),
                &en_de(
                    "The index of the row the height selects, from 0; `null` where none \
                     does, no rows are given or the section is empty.",
                    "Der Index der Zeile, die die Höhe wählt, ab 0; `null`, wo keine es \
                     tut, keine Zeilen angegeben sind oder der Querschnitt leer ist.",
                ),
            ),
            field(
                "required_area",
                AREA,
                &en_de("Required area", "Verlangte Fläche"),
                &en_de(
                    "The section area the row requires; `null` where it requires none.",
                    "Die Querschnittsfläche, die die Zeile verlangt; `null`, wo sie keine \
                     verlangt.",
                ),
            ),
            field(
                "required_width",
                LENGTH,
                &en_de("Required width", "Verlangte Breite"),
                &en_de(
                    "The section width the row requires; `null` where it requires none.",
                    "Die Querschnittsbreite, die die Zeile verlangt; `null`, wo sie keine \
                     verlangt.",
                ),
            ),
            field(
                "members",
                MemberFieldKind::Objects,
                &en_de("Stacked spaces", "Gestapelte Räume"),
                &en_de("Every space of the well.", "Jeder Raum des Schachts."),
            ),
        ],
    },
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "zone_checks",
            parameters: &ZONE,
            dimension: None,
            services: ZONE_SERVICES,
            exactness: MeasuredExactness::Measured,
            subject: MeasuredSubject::Object,
            not_evaluated: &[
                "whether a reached object is a host is undecided",
                "the opening or its host is no straight extrusion the body set bounds",
                "its area may be below the minimum",
                "it may lie in a zone, or a dimension or support may be missed",
            ],
            label: &en_de("Zone checks", "Prüfungen der Zone"),
            help: &en_de(
                "What `opening-zone` checks of the opening in each host its path reaches, in its order, host by host: the placement (within the host's face and outline, its clear distances from the ends and edges and from the bottom and top edges), the allowed zone it misses least, each row of `dimensions` whose `source` selects it, each requirement on the host's supports, and the nearest other opening of the host closer than `opening_spacing`. A distance measured to a free outline from the box the opening may lie in is known only from below.",
                "Was `opening-zone` an der Öffnung in jedem Wirt prüft, den ihr Pfad erreicht, in seiner Reihenfolge, Wirt für Wirt: die Lage (innerhalb der Ansicht und des Umrisses des Wirts, ihre lichten Abstände zu den Enden und Rändern und zum unteren und oberen Rand), die am wenigsten verfehlte erlaubte Zone, jede Zeile der `dimensions`, deren `source` sie wählt, jede Anforderung an die Auflager des Wirts, und die nächste andere Öffnung des Wirts, die näher ist als `opening_spacing`. Ein Abstand zu einem freien Umriss aus dem Quader, in dem die Öffnung liegen kann, ist nur von unten bekannt.",
            ),
        },
        fields: &[
            field(
                "check",
                MemberFieldKind::Text,
                &en_de("Check", "Prüfung"),
                &en_de(
                    "What the item checks: `placement`, `zones`, `dimension`, `support` or `spacing`.",
                    "Was das Element prüft: `placement`, `zones`, `dimension`, `support` oder `spacing`.",
                ),
            ),
            field(
                "placed",
                TRUTH,
                &en_de("Placed", "Platziert"),
                &en_de(
                    "Whether the opening is placed in the host; undecided where it cannot be.",
                    "Ob die Öffnung im Wirt platziert ist; unentschieden, wo sie es nicht sein kann.",
                ),
            ),
            field(
                "host",
                MemberFieldKind::Text,
                &en_de("Host", "Wirt"),
                &en_de("The host's name.", "Der Name des Wirts."),
            ),
            field(
                "inside",
                TRUTH,
                &en_de("Inside", "Innerhalb"),
                &en_de(
                    "Whether the opening lies within the host's face and outline.",
                    "Ob die Öffnung innerhalb der Ansicht und des Umrisses des Wirts liegt.",
                ),
            ),
            field(
                "outside",
                MemberFieldKind::Text,
                &en_de("Outside", "Außerhalb"),
                &en_de(
                    "Where it lies outside, as a finding words it.",
                    "Wo sie außerhalb liegt, wie ein Befund es formuliert.",
                ),
            ),
            field(
                "end",
                LENGTH,
                &en_de("End distance", "Endabstand"),
                &en_de(
                    "The clear distance from the nearer end of the host.",
                    "Der lichte Abstand zum näheren Ende des Wirts.",
                ),
            ),
            field(
                "end_shown",
                MemberFieldKind::Text,
                &en_de("End distance shown", "Endabstand gezeigt"),
                &en_de(
                    "The end distance as a finding shows it.",
                    "Der Endabstand, wie ein Befund ihn zeigt.",
                ),
            ),
            field(
                "edge",
                LENGTH,
                &en_de("Edge distance", "Randabstand"),
                &en_de(
                    "The clear distance from the nearer edge (or flange), negative where it reaches into a flange.",
                    "Der lichte Abstand zum näheren Rand (oder Flansch), negativ, wo sie in einen Flansch reicht.",
                ),
            ),
            field(
                "edge_words",
                MemberFieldKind::Text,
                &en_de("Edge words", "Randworte"),
                &en_de(
                    "The edge distance as a finding words it.",
                    "Der Randabstand, wie ein Befund ihn formuliert.",
                ),
            ),
            field(
                "bottom",
                LENGTH,
                &en_de("Bottom distance", "Abstand unten"),
                &en_de(
                    "The distance from the host's bottom edge (or lower flange), where the rule bounds it.",
                    "Der Abstand zum unteren Rand (oder unteren Flansch) des Wirts, wo die Regel ihn begrenzt.",
                ),
            ),
            field(
                "bottom_shown",
                MemberFieldKind::Text,
                &en_de("Bottom distance shown", "Abstand unten gezeigt"),
                &en_de(
                    "The bottom distance as a finding shows it.",
                    "Der untere Abstand, wie ein Befund ihn zeigt.",
                ),
            ),
            field(
                "bottom_name",
                MemberFieldKind::Text,
                &en_de("Bottom edge", "Unterer Rand"),
                &en_de(
                    "What the bottom distance is measured to.",
                    "Wozu der untere Abstand gemessen wird.",
                ),
            ),
            field(
                "top",
                LENGTH,
                &en_de("Top distance", "Abstand oben"),
                &en_de(
                    "The distance from the host's top edge (or upper flange), where the rule bounds it.",
                    "Der Abstand zum oberen Rand (oder oberen Flansch) des Wirts, wo die Regel ihn begrenzt.",
                ),
            ),
            field(
                "top_shown",
                MemberFieldKind::Text,
                &en_de("Top distance shown", "Abstand oben gezeigt"),
                &en_de(
                    "The top distance as a finding shows it.",
                    "Der obere Abstand, wie ein Befund ihn zeigt.",
                ),
            ),
            field(
                "top_name",
                MemberFieldKind::Text,
                &en_de("Top edge", "Oberer Rand"),
                &en_de(
                    "What the top distance is measured to.",
                    "Wozu der obere Abstand gemessen wird.",
                ),
            ),
            field(
                "far_open",
                TRUTH,
                &en_de("Far edges open", "Ferne Ränder offen"),
                &en_de(
                    "Undecided where a distance known only from below may exceed `edge_distance_maximum`, naming every such edge; false otherwise.",
                    "Unentschieden, wo ein nur von unten bekannter Abstand `edge_distance_maximum` überschreiten kann, mit jedem solchen Rand; sonst falsch.",
                ),
            ),
            field(
                "zone",
                LENGTH,
                &en_de("Clear distance", "Lichter Abstand"),
                &en_de(
                    "The clear distance from the side the opening misses most; `null` where it lies in a zone.",
                    "Der lichte Abstand zur am meisten verfehlten Seite; `null`, wo sie in einer Zone liegt.",
                ),
            ),
            field(
                "needed",
                LENGTH,
                &en_de("Inset", "Abstand"),
                &en_de(
                    "The inset that side needs.",
                    "Der Abstand, den diese Seite braucht.",
                ),
            ),
            field(
                "words",
                MemberFieldKind::Text,
                &en_de("Words", "Worte"),
                &en_de(
                    "The finding as the capability words it.",
                    "Der Befund, wie die Fähigkeit ihn formuliert.",
                ),
            ),
            field(
                "distance",
                LENGTH,
                &en_de("Distance", "Abstand"),
                &en_de(
                    "The distance, from below where a target may be nearer or the outline is known only within bounds.",
                    "Der Abstand, von unten, wo ein Ziel näher sein kann oder der Umriss nur in Grenzen bekannt ist.",
                ),
            ),
            field(
                "minimum",
                LENGTH,
                &en_de("Minimum", "Minimum"),
                &en_de(
                    "The least distance the row allows; `null` for none.",
                    "Der kleinste Abstand, den die Zeile erlaubt; `null` für keinen.",
                ),
            ),
            field(
                "maximum",
                LENGTH,
                &en_de("Maximum", "Maximum"),
                &en_de(
                    "The greatest distance the row allows; `null` for none.",
                    "Der größte Abstand, den die Zeile erlaubt; `null` für keinen.",
                ),
            ),
            field(
                "slack",
                LENGTH,
                &en_de("Tolerance", "Toleranz"),
                &en_de(
                    "The row's tolerance with the rounding of placements.",
                    "Die Toleranz der Zeile mit der Rundung der Lagen.",
                ),
            ),
            field(
                "required",
                MemberFieldKind::Text,
                &en_de("Required", "Gefordert"),
                &en_de(
                    "What the row requires, as a finding words it.",
                    "Was die Zeile fordert, wie ein Befund es formuliert.",
                ),
            ),
            field(
                "label",
                MemberFieldKind::Text,
                &en_de("Row", "Zeile"),
                &en_de("The row's name.", "Der Name der Zeile."),
            ),
            field(
                "open",
                MemberFieldKind::Text,
                &en_de("Doubt", "Zweifel"),
                &en_de(
                    "Why the row may be missed, where it is not decided.",
                    "Warum die Zeile verfehlt sein kann, wo sie nicht entschieden ist.",
                ),
            ),
            field(
                "fails",
                TRUTH,
                &en_de("Fails", "Verfehlt"),
                &en_de(
                    "Whether a support surely misses the requirement; undecided where one may.",
                    "Ob ein Auflager die Anforderung sicher verfehlt; unentschieden, wo eines es kann.",
                ),
            ),
            field(
                "message",
                MemberFieldKind::Text,
                &en_de("Message", "Meldung"),
                &en_de(
                    "The finding as the capability words it.",
                    "Der Befund, wie die Fähigkeit ihn formuliert.",
                ),
            ),
            field(
                "spacing",
                LENGTH,
                &en_de("Spacing", "Abstand zu Öffnungen"),
                &en_de(
                    "The least clear distance to another opening of the host surely closer than `opening_spacing`; undecided where one may be.",
                    "Der kleinste lichte Abstand zu einer anderen Öffnung des Wirts, die sicher näher ist als `opening_spacing`; unentschieden, wo eine es sein kann.",
                ),
            ),
            field(
                "related",
                MemberFieldKind::Objects,
                &en_de("Related", "Bezogen"),
                &en_de(
                    "What a finding on the item relates: the host, and the objects it is measured to.",
                    "Worauf sich ein Befund zum Element bezieht: den Wirt und die Objekte, zu denen gemessen wurde.",
                ),
            ),
            field(
                "reason",
                MemberFieldKind::Text,
                &en_de("Reason", "Grund"),
                &en_de(
                    "Why the item is undecided where not for incomplete evidence, as a report writes it (`missing_service`).",
                    "Warum das Element unentschieden ist, wo nicht wegen unvollständiger Belege, wie ein Bericht es schreibt (`missing_service`).",
                ),
            ),
        ],
    },
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
