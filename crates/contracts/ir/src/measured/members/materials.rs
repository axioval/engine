//! The member list `layers`: the material layers of a wall, a slab or a
//! roof as its source states them, inside first where the inside can be
//! decided from the spaces the element bounds.

use super::super::registry::en_de;
use super::super::{MeasuredDescriptor, MeasuredExactness, MeasuredSubject};
use super::{LENGTH, MemberDescriptor, MemberFieldKind, RATIO, TRUTH, field, optional_kinds};

const TEXT: MemberFieldKind = MemberFieldKind::Text;

pub(super) const LAYERS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "layers",
        parameters: &[optional_kinds(
            "spaces",
            &en_de(
                "The source kinds of the spaces the element bounds, `,`-separated; subtypes \
                 match. The inside is the face beside which such spaces lie when none lies \
                 beside the other; without it the inside is not decided.",
                "Die Quellarten der Räume, die das Bauteil begrenzt, durch `,` getrennt; \
                 Untertypen zählen mit. Innen ist die Seite, an der solche Räume liegen, wenn \
                 an der anderen keiner liegt; ohne Angabe ist innen nicht entschieden.",
            ),
        )],
        dimension: None,
        services: &[
            "material-layer",
            "object-frame",
            "relationship-selection",
            "type-hierarchy",
        ],
        exactness: MeasuredExactness::Measured,
        subject: MeasuredSubject::Object,
        not_evaluated: &[
            "the element's material is not a layer set (a constituent, profile or single \
             material, or a list), or cannot be read",
            "a layer's thickness is stated in a unit that cannot be resolved (its thickness \
             and every distance from the inside it enters)",
            "the inside is not decided: no space kinds are given, the layer set states no \
             direction, the element's frame cannot be read, the spaces beside its faces \
             cannot be derived, spaces lie beside both faces or beside neither, or the \
             layers run along the faces (each layer's position and distance from the \
             inside)",
        ],
        label: &en_de("Material layers", "Materialschichten"),
        help: &en_de(
            "The material layers of the element as its source states them, inside first \
             where the inside is decided, otherwise in the source's order: each with its \
             material, thickness, place from the inside and distance from the inner face. \
             Sum `thickness / λ` with λ from a catalogue for a thermal resistance; relate \
             neighbouring layers with `pairs` by `position`.",
            "Die Materialschichten des Bauteils, wie seine Quelle sie angibt, von innen \
             nach außen, wo innen entschieden ist, sonst in der Reihenfolge der Quelle: \
             jede mit Material, Dicke, Platz von innen und Abstand von der Innenseite. \
             Die Summe von `thickness / λ` mit λ aus einem Katalog ergibt einen \
             Wärmedurchlasswiderstand; benachbarte Schichten verknüpft `pairs` nach \
             `position`.",
        ),
    },
    fields: &[
        field(
            "layer",
            TEXT,
            &en_de("Layer", "Schicht"),
            &en_de(
                "The layer's identity in its source and its material, as findings name it.",
                "Die Identität der Schicht in ihrer Quelle und ihr Material, wie Befunde \
                 sie nennen.",
            ),
        ),
        field(
            "index",
            RATIO,
            &en_de("Index", "Nummer"),
            &en_de(
                "The layer's place in the source's order, from 1.",
                "Der Platz der Schicht in der Reihenfolge der Quelle, ab 1.",
            ),
        ),
        field(
            "position",
            RATIO,
            &en_de("Position from inside", "Platz von innen"),
            &en_de(
                "The layer's place counted from the inside, from 1; not evaluated where \
                 the inside is not decided.",
                "Der Platz der Schicht von innen gezählt, ab 1; nicht ausgewertet, wo innen \
                 nicht entschieden ist.",
            ),
        ),
        field(
            "material",
            TEXT,
            &en_de("Material", "Material"),
            &en_de(
                "The name of the layer's material; `null` where the layer states none, \
                 such as an air gap.",
                "Der Name des Materials der Schicht; `null`, wo die Schicht keines angibt, \
                 etwa eine Luftschicht.",
            ),
        ),
        field(
            "layer_name",
            TEXT,
            &en_de("Layer name", "Schichtname"),
            &en_de(
                "The layer's own name; `null` where it states none.",
                "Der eigene Name der Schicht; `null`, wo sie keinen angibt.",
            ),
        ),
        field(
            "layer_category",
            TEXT,
            &en_de("Layer category", "Schichtkategorie"),
            &en_de(
                "The layer's category, its role in the set (`LoadBearing`, `Insulation`); \
                 `null` where it states none.",
                "Die Kategorie der Schicht, ihre Rolle im Aufbau (`LoadBearing`, \
                 `Insulation`); `null`, wo sie keine angibt.",
            ),
        ),
        field(
            "thickness",
            LENGTH,
            &en_de("Thickness", "Dicke"),
            &en_de(
                "The layer's stated thickness.",
                "Die angegebene Dicke der Schicht.",
            ),
        ),
        field(
            "from_inside",
            LENGTH,
            &en_de("Distance from inside", "Abstand von innen"),
            &en_de(
                "The summed thickness of the layers between the inner face and this layer; \
                 zero for the innermost, not evaluated where the inside is not decided.",
                "Die Summe der Dicken der Schichten zwischen der Innenseite und dieser \
                 Schicht; null für die innerste, nicht ausgewertet, wo innen nicht \
                 entschieden ist.",
            ),
        ),
        field(
            "ventilated",
            TRUTH,
            &en_de("Ventilated", "Belüftet"),
            &en_de(
                "Whether the layer is a ventilated air gap; `null` where the source states \
                 neither, or that it is unknown.",
                "Ob die Schicht eine belüftete Luftschicht ist; `null`, wo die Quelle weder \
                 das eine noch das andere angibt oder es als unbekannt angibt.",
            ),
        ),
        field(
            "priority",
            RATIO,
            &en_de("Priority", "Priorität"),
            &en_de(
                "The layer's stated priority at connections (0 to 100); `null` where it \
                 states none.",
                "Die angegebene Priorität der Schicht an Anschlüssen (0 bis 100); `null`, wo \
                 sie keine angibt.",
            ),
        ),
    ],
};
