//! Measured member lists: the parts of an object built-in code measures
//! one by one (a flight's steps, a ramp's runs), which an aggregate ranges
//! over and whose fields its expressions read in [`MEMBER_SET`](crate::MEMBER_SET).
//!
//! A list is written like a measured value, `name[;key=value…]`, and
//! parsed against [`MEASURED_MEMBERS`] with [`parse_members`].

use serde::Serialize;

use super::registry::en_de;
use super::{
    LocalizedText, MeasuredCall, MeasuredDescriptor, MeasuredError, MeasuredExactness,
    MeasuredParameter, MeasuredParameterKind,
};
use crate::QuantityDimension;

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
}

const FLIGHTS: &[&str] = &["walking-surface"];

const WALKING_LINE_OFFSET: MeasuredParameter = MeasuredParameter {
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

const fn required_length(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: true,
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
    MemberDescriptor {
        list: MeasuredDescriptor {
            name: "runs",
            parameters: &[],
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
        ],
    },
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
                name: "open_riser",
                kind: MemberFieldKind::Truth,
                label: &en_de("Open riser", "Offene Steigung"),
                help: &en_de(
                    "Whether the riser leaves the step open.",
                    "Ob die Steigung die Stufe offen lässt.",
                ),
            },
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
