//! The member list `zone_neighbours` (#319): the context zones beside the
//! zone an object stands in, or beside a zone named by id, such as a
//! parcel's neighbouring parcels and the street in front of it.

use super::super::registry::{
    ZONE_LINE, ZONE_OF_OBJECT, ZONE_SERVICES, ZONE_STANDS_UNDECIDED, ZONE_UNPLACED,
    ZONE_UNSUPPLIED, en_de,
};
use super::super::{
    MeasuredDescriptor, MeasuredExactness, MeasuredParameter, MeasuredParameterKind,
    MeasuredSubject,
};
use super::{AREA, MemberDescriptor, MemberFieldKind, field};

const TEXT: MemberFieldKind = MemberFieldKind::Text;

pub(super) const ZONE_NEIGHBOURS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "zone_neighbours",
        parameters: &[
            ZONE_OF_OBJECT[0],
            ZONE_OF_OBJECT[1],
            MeasuredParameter {
                key: "kinds",
                kind: MeasuredParameterKind::Text,
                required: false,
                default: None,
                help: &en_de(
                    "The kinds of the neighbours listed, `,`-separated, such as \
                     `street,way,square` for the public areas beside a parcel; without it, \
                     every kind.",
                    "Die Arten der aufgeführten Nachbarn, durch `,` getrennt, etwa \
                     `street,way,square` für die öffentlichen Flächen neben einem Flurstück; \
                     ohne Angabe jede Art.",
                ),
            },
        ],
        dimension: None,
        services: ZONE_SERVICES,
        exactness: MeasuredExactness::Measured,
        subject: MeasuredSubject::Object,
        not_evaluated: &[
            ZONE_UNSUPPLIED,
            ZONE_UNPLACED,
            ZONE_STANDS_UNDECIDED,
            ZONE_LINE,
            "a zone of the kinds listed cannot be placed",
        ],
        label: &en_de("Neighbouring zones", "Benachbarte Zonen"),
        help: &en_de(
            "The supplied areas beside a context zone (the one the object stands in, or one \
             named by id), in the order supplied: a zone whose stated boundary shares a \
             stretch with the zone's is surely a neighbour, one whose boundary only comes \
             within their accuracies of it possibly one, and one farther away none. Count \
             `kinds=street,way,square` for the public areas a parcel borders.",
            "Die gelieferten Flächen neben einer Kontextzone (der, in der das Objekt steht, \
             oder einer mit Kennung genannten), in der gelieferten Reihenfolge: eine Zone, \
             deren angegebener Rand ein Stück mit dem der Zone teilt, ist sicher ein Nachbar, \
             eine, deren Rand ihr nur innerhalb ihrer Genauigkeiten nahekommt, möglicherweise, \
             und eine weiter entfernte keiner. `kinds=street,way,square` zählt die \
             öffentlichen Flächen, an die ein Flurstück grenzt.",
        ),
    },
    fields: &[
        field(
            "zone",
            TEXT,
            &en_de("Zone", "Zone"),
            &en_de(
                "The neighbour's id, as its file states it.",
                "Die Kennung des Nachbarn, wie seine Datei sie angibt.",
            ),
        ),
        field(
            "kind",
            TEXT,
            &en_de("Kind", "Art"),
            &en_de(
                "The neighbour's kind, such as `parcel` or `street`.",
                "Die Art des Nachbarn, etwa `parcel` oder `street`.",
            ),
        ),
        field(
            "area",
            AREA,
            &en_de("Area", "Fläche"),
            &en_de(
                "The neighbour's area, an interval holding every boundary its accuracy \
                 allows.",
                "Die Fläche des Nachbarn, ein Intervall, das jeden Rand hält, den seine \
                 Genauigkeit zulässt.",
            ),
        ),
    ],
};
