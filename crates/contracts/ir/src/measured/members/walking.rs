//! The member lists `stair-geometry` and `ramp-geometry` judge item by
//! item: the clearances, landings, doors, end spaces, clear widths and
//! handrails of a flight or a ramp, each a measurement or a search's
//! three-valued answer per item.

use super::super::registry::en_de;
use super::super::{
    LocalizedText, MeasuredDescriptor, MeasuredExactness, MeasuredParameter, MeasuredParameterKind,
};
use super::{
    FLIGHTS, LENGTH, MemberDescriptor, MemberField, MemberFieldKind, RATIO, TRUTH,
    WALKING_LINE_OFFSET, field,
};

const TEXT: MemberFieldKind = MemberFieldKind::Text;
const OBJECTS: MemberFieldKind = MemberFieldKind::Objects;

const OF: MeasuredParameter = MeasuredParameter {
    key: "of",
    kind: MeasuredParameterKind::Choice {
        options: &["flight", "ramp"],
    },
    required: false,
    default: Some("flight"),
    help: &en_de(
        "Whether the object is a stair flight or a ramp, measured in its runs.",
        "Ob das Objekt ein Treppenlauf oder eine Rampe ist, in ihren Läufen gemessen.",
    ),
};

const fn objects(
    key: &'static str,
    required: bool,
    help: &'static [LocalizedText],
) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Objects,
        required,
        default: None,
        help,
    }
}

const fn length(
    key: &'static str,
    required: bool,
    help: &'static [LocalizedText],
) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required,
        default: None,
        help,
    }
}

const fn list(
    name: &'static str,
    parameters: &'static [MeasuredParameter],
    services: &'static [&'static str],
    not_evaluated: &'static [&'static str],
    label: &'static [LocalizedText],
    help: &'static [LocalizedText],
) -> MeasuredDescriptor {
    MeasuredDescriptor {
        name,
        parameters,
        dimension: None,
        services,
        exactness: MeasuredExactness::Measured,
        not_evaluated,
        label,
        help,
    }
}

const LANDING: MeasuredParameter = objects(
    "landing",
    true,
    &en_de(
        "The objects that may carry a landing: their source kinds, `,`-separated, or `@` a \
         selector parameter of the rule.",
        "Die Objekte, die ein Podest tragen können: ihre Quellarten, durch `,` getrennt, oder \
         mit `@` ein Selektorparameter der Regel.",
    ),
);

const LABEL: MemberField = field(
    "label",
    TEXT,
    &en_de("Label", "Bezeichnung"),
    &en_de(
        "How messages name the item: `the bottom of run 1 of 2`, `run 2 of 3`.",
        "Wie Meldungen das Element nennen: `the bottom of run 1 of 2`, `run 2 of 3`.",
    ),
);

const SCALE: MemberField = field(
    "scale",
    RATIO,
    &en_de("Scale", "Größenordnung"),
    &en_de(
        "The largest magnitude among the positions measured, in metres: the binary rounding \
         of decimal coordinates grows with it.",
        "Der größte Betrag unter den gemessenen Lagen, in Metern: die binäre Rundung \
         dezimaler Koordinaten wächst mit ihm.",
    ),
);

const FOUND: MemberField = field(
    "found",
    TRUTH,
    &en_de("Found", "Gefunden"),
    &en_de(
        "Whether the search found what it looks for; undecided, with why, where it cannot \
         tell, including where an object the selection could not decide may be it.",
        "Ob die Suche fand, wonach sie sucht; unentschieden mit Grund, wo sie es nicht \
         entscheiden kann, auch wo ein Objekt unentschiedener Auswahl es sein kann.",
    ),
);

const FINDING: MemberField = field(
    "finding",
    TEXT,
    &en_de("Finding", "Befund"),
    &en_de(
        "What the search found, in words; empty where it found nothing.",
        "Was die Suche fand, in Worten; leer, wo sie nichts fand.",
    ),
);

const FOUND_OBJECTS: MemberField = field(
    "objects",
    OBJECTS,
    &en_de("Objects", "Objekte"),
    &en_de(
        "The objects involved in what the search found.",
        "Die Objekte, die an dem Gefundenen beteiligt sind.",
    ),
);

const SEARCH: &[MemberField] = &[FOUND, FINDING, FOUND_OBJECTS, LABEL];

const STRETCH: MemberField = field(
    "stretch",
    RATIO,
    &en_de("Stretch", "Abschnitt"),
    &en_de(
        "The flight (1) or run the item lies along, counted from 1 at the lowest.",
        "Der Lauf (1) oder Rampenlauf, an dem das Element liegt, ab 1 beim untersten gezählt.",
    ),
);

const RAILS: &[MeasuredParameter] = &[
    OF,
    WALKING_LINE_OFFSET,
    objects(
        "rails",
        true,
        &en_de(
            "The objects that may be handrails: their source kinds, `,`-separated, or `@` a \
             selector parameter of the rule.",
            "Die Objekte, die Handläufe sein können: ihre Quellarten, durch `,` getrennt, oder \
             mit `@` ein Selektorparameter der Regel.",
        ),
    ),
    length(
        "reach_across",
        true,
        &en_de(
            "How far outside the walking surface's sides a rail may run.",
            "Wie weit außerhalb der Seiten der Lauffläche ein Handlauf liegen darf.",
        ),
    ),
    length(
        "reach_above",
        true,
        &en_de(
            "How far above the pitch line a rail may run.",
            "Wie weit über der Steigungslinie ein Handlauf liegen darf.",
        ),
    ),
    length(
        "level_over",
        false,
        &en_de(
            "The extension over which a rail's rise beyond each end is measured; without it, \
             none.",
            "Die Verlängerung, über die der Anstieg eines Handlaufs an jedem Ende gemessen \
             wird; ohne Angabe keine.",
        ),
    ),
    MeasuredParameter {
        key: "from",
        kind: MeasuredParameterKind::Choice {
            options: &["nosing", "riser"],
        },
        required: false,
        default: None,
        help: &en_de(
            "Whether a flight's extensions are measured from its first and last nosing (the \
             default) or riser.",
            "Ob die Verlängerungen eines Laufs von der ersten und letzten Stufenvorderkante \
             (Vorgabe) oder Steigung gemessen werden.",
        ),
    },
];

const RAIL_UNMEASURED: &[&str] = &[
    "the walking-surface service cannot measure the flight or the ramp's runs",
    "a selector parameter it names picks objects it cannot list",
];

/// `clear_widths`.
pub(super) const CLEAR_WIDTHS: MemberDescriptor = MemberDescriptor {
    list: list(
        "clear_widths",
        &[
            OF,
            WALKING_LINE_OFFSET,
            objects(
                "obstacles",
                true,
                &en_de(
                    "The objects that may narrow the walking width: their source kinds, \
                     `,`-separated, or `@` a selector parameter of the rule.",
                    "Die Objekte, die die Laufbreite einengen können: ihre Quellarten, durch \
                     `,` getrennt, oder mit `@` ein Selektorparameter der Regel.",
                ),
            ),
            length(
                "band_from",
                true,
                &en_de(
                    "The bottom of the band the width is measured in, above the pitch line.",
                    "Die Unterkante des Bandes, in dem die Breite gemessen wird, über der \
                     Steigungslinie.",
                ),
            ),
            length(
                "band_to",
                true,
                &en_de("The top of that band.", "Die Oberkante dieses Bandes."),
            ),
            objects(
                "landing",
                false,
                &en_de(
                    "With a flight, the objects that may carry a landing at its ends, whose \
                     clear widths are listed too.",
                    "Bei einem Lauf die Objekte, die an seinen Enden ein Podest tragen \
                     können, dessen lichte Breiten ebenfalls aufgeführt werden.",
                ),
            ),
            MeasuredParameter {
                key: "ends",
                kind: MeasuredParameterKind::Choice {
                    options: &["both", "bottom", "top", "none"],
                },
                required: false,
                default: Some("both"),
                help: &en_de(
                    "Which ends' landings are listed.",
                    "Welche Enden ihre Podeste aufführen.",
                ),
            },
            MeasuredParameter {
                key: "stretch",
                kind: MeasuredParameterKind::Choice {
                    options: &["yes", "no"],
                },
                required: false,
                default: Some("yes"),
                help: &en_de(
                    "Whether the flight itself is listed.",
                    "Ob der Lauf selbst aufgeführt wird.",
                ),
            },
        ],
        FLIGHTS,
        &[
            "the walking-surface service cannot measure the flight or the ramp's runs",
            "a selector parameter it names picks objects it cannot list",
        ],
        &en_de("Clear widths", "Lichte Breiten"),
        &en_de(
            "The clear width of a flight and the landings at its ends, or of each of a \
             ramp's runs, between two heights above the pitch line or the landing's level, \
             as `stair-geometry` and `ramp-geometry` measure them.",
            "Die lichte Breite eines Laufs und der Podeste an seinen Enden, oder jedes Laufs \
             einer Rampe, zwischen zwei Höhen über der Steigungslinie oder dem Podestniveau, \
             wie `stair-geometry` und `ramp-geometry` sie messen.",
        ),
    ),
    fields: &[
        LABEL,
        field(
            "above",
            TEXT,
            &en_de("Above", "Über"),
            &en_de(
                "What the band stands on in words: `its pitch line`, `its level`.",
                "Worauf das Band steht, in Worten: `its pitch line`, `its level`.",
            ),
        ),
        field(
            "width",
            LENGTH,
            &en_de("Clear width", "Lichte Breite"),
            &en_de(
                "The narrowest free width; undecided, with why, where it is not measured.",
                "Die engste freie Breite; unentschieden mit Grund, wo sie nicht gemessen ist.",
            ),
        ),
        field(
            "governing",
            OBJECTS,
            &en_de("Bounded by", "Begrenzt durch"),
            &en_de(
                "The obstacles bounding its narrowest place; none where its own sides do.",
                "Die Hindernisse an seiner engsten Stelle; keine, wo seine eigenen Seiten \
                 sie begrenzen.",
            ),
        ),
        field(
            "place",
            TEXT,
            &en_de("Place", "Ort"),
            &en_de(
                "`stretch` for a flight or run, `landing` for a landing.",
                "`stretch` für einen Lauf, `landing` für ein Podest.",
            ),
        ),
    ],
};

/// `clearances`.
pub(super) const CLEARANCES: MemberDescriptor = MemberDescriptor {
    list: list(
        "clearances",
        &[
            OF,
            MeasuredParameter {
                key: "side",
                kind: MeasuredParameterKind::Choice {
                    options: &["above", "below"],
                },
                required: false,
                default: Some("above"),
                help: &en_de(
                    "Headroom above the walking surface, or the clearance below the flight \
                     or ramp over the floors of the spaces under it.",
                    "Kopfhöhe über der Lauffläche, oder die lichte Höhe unter dem Lauf oder \
                     der Rampe über den Böden der Räume darunter.",
                ),
            },
            objects(
                "obstacles",
                true,
                &en_de(
                    "Above, the objects that may lower the headroom; below, the spaces whose \
                     floors count: their source kinds, `,`-separated, or `@` a selector \
                     parameter of the rule.",
                    "Oben die Objekte, die die Kopfhöhe senken können; unten die Räume, \
                     deren Böden zählen: ihre Quellarten, durch `,` getrennt, oder mit `@` \
                     ein Selektorparameter der Regel.",
                ),
            ),
        ],
        FLIGHTS,
        &["the walking-surface service cannot measure the clearance"],
        &en_de("Clearances", "Lichte Höhen"),
        &en_de(
            "The headroom above a flight's or ramp's walking surface, or the clearance under \
             it, as one item with the objects governing it, as `stair-geometry` and \
             `ramp-geometry` measure it.",
            "Die Kopfhöhe über der Lauffläche eines Laufs oder einer Rampe, oder die lichte \
             Höhe darunter, als ein Element mit den maßgebenden Objekten, wie \
             `stair-geometry` und `ramp-geometry` sie messen.",
        ),
    ),
    fields: &[
        field(
            "clearance",
            LENGTH,
            &en_de("Clearance", "Lichte Höhe"),
            &en_de(
                "The least clearance; `null` where nothing selected stands above (or below).",
                "Die kleinste lichte Höhe; `null`, wo nichts Ausgewähltes darüber (oder \
                 darunter) steht.",
            ),
        ),
        field(
            "governing",
            OBJECTS,
            &en_de("Governed by", "Maßgebend"),
            &en_de(
                "The objects at the least clearance.",
                "Die Objekte an der kleinsten lichten Höhe.",
            ),
        ),
        field(
            "noun",
            TEXT,
            &en_de("Noun", "Bezeichnung"),
            &en_de("`flight` or `ramp`.", "`flight` oder `ramp`."),
        ),
    ],
};

/// `end_spaces`.
pub(super) const END_SPACES: MemberDescriptor = MemberDescriptor {
    list: list(
        "end_spaces",
        &[
            OF,
            WALKING_LINE_OFFSET,
            objects(
                "obstacles",
                true,
                &en_de(
                    "The objects that may obstruct the space: their source kinds, \
                     `,`-separated, or `@` a selector parameter of the rule.",
                    "Die Objekte, die den Raum versperren können: ihre Quellarten, durch \
                     `,` getrennt, oder mit `@` ein Selektorparameter der Regel.",
                ),
            ),
            length(
                "depth",
                true,
                &en_de("The free space's depth.", "Die Tiefe des freien Raums."),
            ),
            length(
                "width",
                true,
                &en_de("The free space's width.", "Die Breite des freien Raums."),
            ),
            length(
                "height",
                true,
                &en_de("The free space's height.", "Die Höhe des freien Raums."),
            ),
        ],
        &["walking-surface", "free-space"],
        RAIL_UNMEASURED,
        &en_de("End spaces", "Freiräume an den Enden"),
        &en_de(
            "Whether a selected object reaches into the free space before a flight's first \
             riser and beyond its last, or in front of a ramp's lowest and beyond its \
             highest run: one search per end, `bottom` and `top`.",
            "Ob ein ausgewähltes Objekt in den freien Raum vor der ersten und hinter der \
             letzten Steigung eines Laufs, oder vor dem untersten und hinter dem obersten \
             Lauf einer Rampe reicht: eine Suche je Ende, `bottom` und `top`.",
        ),
    ),
    fields: SEARCH,
};

const DOOR_PARAMETERS: &[MeasuredParameter] = &[
    OF,
    WALKING_LINE_OFFSET,
    LANDING,
    objects(
        "doors",
        true,
        &en_de(
            "The doors: their source kinds, `,`-separated, or `@` a selector parameter of \
             the rule.",
            "Die Türen: ihre Quellarten, durch `,` getrennt, oder mit `@` ein \
             Selektorparameter der Regel.",
        ),
    ),
    length(
        "height",
        true,
        &en_de(
            "The height of the column over a landing a door must reach into.",
            "Die Höhe der Säule über einem Podest, in die eine Tür reichen muss.",
        ),
    ),
];

/// `landing_doors`.
pub(super) const LANDING_DOORS: MemberDescriptor = MemberDescriptor {
    list: list(
        "landing_doors",
        DOOR_PARAMETERS,
        &["walking-surface", "free-space"],
        RAIL_UNMEASURED,
        &en_de("Doors on landings", "Türen auf Podesten"),
        &en_de(
            "Whether a selected door stands on the landing at each end of a flight or of \
             each of a ramp's runs: one search per landing measured.",
            "Ob eine ausgewählte Tür auf dem Podest an jedem Ende eines Laufs oder jedes \
             Laufs einer Rampe steht: eine Suche je gemessenem Podest.",
        ),
    ),
    fields: SEARCH,
};

/// `landing_swings`.
pub(super) const LANDING_SWINGS: MemberDescriptor = MemberDescriptor {
    list: list(
        "landing_swings",
        DOOR_PARAMETERS,
        &["walking-surface", "object-frame", "vertical-extent"],
        RAIL_UNMEASURED,
        &en_de(
            "Doors swinging over landings",
            "Über Podeste schlagende Türen",
        ),
        &en_de(
            "Whether a selected door's leaves swing over the landing at each end of a flight \
             or of each of a ramp's runs: one search per landing measured.",
            "Ob die Flügel einer ausgewählten Tür über das Podest an jedem Ende eines Laufs \
             oder jedes Laufs einer Rampe schlagen: eine Suche je gemessenem Podest.",
        ),
    ),
    fields: SEARCH,
};

/// `landings`.
pub(super) const LANDINGS: MemberDescriptor = MemberDescriptor {
    list: list(
        "landings",
        &[OF, WALKING_LINE_OFFSET, LANDING],
        FLIGHTS,
        &["the walking-surface service cannot measure the flight or the ramp's runs"],
        &en_de("Landings", "Podeste"),
        &en_de(
            "The landing at each end of a flight, or at both ends of each of a ramp's runs, \
             lowest first, as `stair-geometry` and `ramp-geometry` measure them.",
            "Das Podest an jedem Ende eines Laufs, oder an beiden Enden jedes Laufs einer \
             Rampe, das unterste zuerst, wie `stair-geometry` und `ramp-geometry` sie messen.",
        ),
    ),
    fields: &[
        LABEL,
        field(
            "noun",
            TEXT,
            &en_de("Noun", "Bezeichnung"),
            &en_de("`flight` or `run`.", "`flight` oder `run`."),
        ),
        field(
            "present",
            TRUTH,
            &en_de("Present", "Vorhanden"),
            &en_de(
                "Whether a selected object carries a landing there; undecided, with why, \
                 where the landing is not measured.",
                "Ob ein ausgewähltes Objekt dort ein Podest trägt; unentschieden mit Grund, \
                 wo das Podest nicht gemessen ist.",
            ),
        ),
        field(
            "depth",
            LENGTH,
            &en_de("Depth", "Tiefe"),
            &en_de(
                "The landing's depth along the walking direction; `null` without a landing, \
                 undecided where it fills no rectangle.",
                "Die Tiefe des Podests in Gehrichtung; `null` ohne Podest, unentschieden, wo \
                 es kein Rechteck füllt.",
            ),
        ),
        field(
            "width",
            LENGTH,
            &en_de("Width", "Breite"),
            &en_de(
                "The landing's width across the walking direction, likewise.",
                "Die Breite des Podests quer zur Gehrichtung, ebenso.",
            ),
        ),
        field(
            "carrier",
            TEXT,
            &en_de("Carrier", "Träger"),
            &en_de(
                "The object carrying the landing, as messages name it; empty without one.",
                "Das Objekt, das das Podest trägt, wie Meldungen es nennen; leer ohne.",
            ),
        ),
        field(
            "carriers",
            OBJECTS,
            &en_de("Carried by", "Getragen von"),
            &en_de(
                "The object carrying the landing, unless it is the flight or ramp itself.",
                "Das Objekt, das das Podest trägt, sofern es nicht der Lauf oder die Rampe \
                 selbst ist.",
            ),
        ),
        field(
            "walking",
            LENGTH,
            &en_de("Walking width", "Laufbreite"),
            &en_de(
                "The width of the flight or run meeting the landing (a turning flight's end \
                 tread); `null` where it is not measured.",
                "Die Breite des Laufs, der auf das Podest trifft (bei einem gewendelten Lauf \
                 die Endstufe); `null`, wo sie nicht gemessen ist.",
            ),
        ),
        field(
            "outermost",
            TRUTH,
            &en_de("Outermost", "Äußerstes"),
            &en_de(
                "Whether it is one of a ramp's two outermost ends.",
                "Ob es eines der beiden äußersten Enden einer Rampe ist.",
            ),
        ),
        SCALE,
    ],
};

/// `handrail_stretches`.
pub(super) const HANDRAIL_STRETCHES: MemberDescriptor = MemberDescriptor {
    list: list(
        "handrail_stretches",
        RAILS,
        FLIGHTS,
        RAIL_UNMEASURED,
        &en_de("Handrail stretches", "Handlaufabschnitte"),
        &en_de(
            "The flight, or each of a ramp's runs, with the sides selected rails run along.",
            "Der Lauf oder jeder Lauf einer Rampe, mit den Seiten, an denen ausgewählte \
             Handläufe verlaufen.",
        ),
    ),
    fields: &[
        STRETCH,
        LABEL,
        field(
            "measured",
            TRUTH,
            &en_de("Measured", "Gemessen"),
            &en_de(
                "True where its handrails are measured; undecided, with why, where not.",
                "Wahr, wo seine Handläufe gemessen sind; unentschieden mit Grund, wo nicht.",
            ),
        ),
        field(
            "sides",
            RATIO,
            &en_de("Sides with a rail", "Seiten mit Handlauf"),
            &en_de(
                "How many of its sides a selected rail runs along: 0, 1 or 2.",
                "An wie vielen seiner Seiten ein ausgewählter Handlauf verläuft: 0, 1 oder 2.",
            ),
        ),
        field(
            "side",
            TEXT,
            &en_de("Side", "Seite"),
            &en_de(
                "Where a rail runs along one side only, `left` or `right` (seen climbing).",
                "Wo ein Handlauf nur an einer Seite verläuft, `left` oder `right` (steigend \
                 gesehen).",
            ),
        ),
        field(
            "on_sides",
            OBJECTS,
            &en_de("Rails on its sides", "Handläufe an den Seiten"),
            &en_de(
                "The rails running along a side.",
                "Die Handläufe, die an einer Seite verlaufen.",
            ),
        ),
        field(
            "width",
            LENGTH,
            &en_de("Width", "Breite"),
            &en_de(
                "Its width; `null` where not measured.",
                "Seine Breite; `null`, wo nicht gemessen.",
            ),
        ),
        SCALE,
    ],
};

/// `rail_heights`.
pub(super) const RAIL_HEIGHTS: MemberDescriptor = MemberDescriptor {
    list: list(
        "rail_heights",
        RAILS,
        FLIGHTS,
        RAIL_UNMEASURED,
        &en_de("Handrail heights", "Handlaufhöhen"),
        &en_de(
            "Each selected rail along the flight or a ramp's runs, its top's height above the \
             pitch line at its lowest and highest.",
            "Jeder ausgewählte Handlauf entlang des Laufs oder der Rampenläufe, die Höhe \
             seiner Oberkante über der Steigungslinie an der niedrigsten und höchsten Stelle.",
        ),
    ),
    fields: &[
        STRETCH,
        LABEL,
        field(
            "rail",
            TEXT,
            &en_de("Rail", "Handlauf"),
            &en_de(
                "The rail, as messages name it.",
                "Der Handlauf, wie Meldungen ihn nennen.",
            ),
        ),
        field(
            "rails",
            OBJECTS,
            &en_de("Rails", "Handläufe"),
            &en_de("The rail.", "Der Handlauf."),
        ),
        field(
            "lowest",
            LENGTH,
            &en_de("Lowest height", "Niedrigste Höhe"),
            &en_de(
                "Its top's least height above the pitch line.",
                "Die kleinste Höhe seiner Oberkante über der Steigungslinie.",
            ),
        ),
        field(
            "highest",
            LENGTH,
            &en_de("Highest height", "Höchste Höhe"),
            &en_de(
                "Its top's greatest height above the pitch line.",
                "Die größte Höhe seiner Oberkante über der Steigungslinie.",
            ),
        ),
        SCALE,
    ],
};

/// `rail_extensions`.
pub(super) const RAIL_EXTENSIONS: MemberDescriptor = MemberDescriptor {
    list: list(
        "rail_extensions",
        RAILS,
        FLIGHTS,
        RAIL_UNMEASURED,
        &en_de("Handrail extensions", "Handlaufverlängerungen"),
        &en_de(
            "How far the handrail along each side reaches beyond each end (its first piece's \
             at the bottom, its last's at the top), and each rail over the middle on its own.",
            "Wie weit der Handlauf an jeder Seite über jedes Ende hinausreicht (am unteren \
             Ende sein erstes Stück, am oberen sein letztes), und jeder Handlauf über der \
             Mitte für sich.",
        ),
    ),
    fields: &[
        STRETCH,
        LABEL,
        field(
            "rail",
            TEXT,
            &en_de("Rail", "Handlauf"),
            &en_de(
                "The rail; empty for a side whose pieces cannot be put in order.",
                "Der Handlauf; leer für eine Seite, deren Stücke sich nicht ordnen lassen.",
            ),
        ),
        field(
            "rails",
            OBJECTS,
            &en_de("Rails", "Handläufe"),
            &en_de("The rail.", "Der Handlauf."),
        ),
        field(
            "end",
            TEXT,
            &en_de("End", "Ende"),
            &en_de("`bottom` or `top`.", "`bottom` oder `top`."),
        ),
        field(
            "other",
            TEXT,
            &en_de("Other part", "Anderer Teil"),
            &en_de(
                "Where a rail runs along another part of a turning flight than this end's: \
                 `a later` or `an earlier`.",
                "Wo ein Handlauf an einem anderen Teil eines gewendelten Laufs als dem dieses \
                 Endes verläuft: `a later` oder `an earlier`.",
            ),
        ),
        field(
            "from",
            TEXT,
            &en_de("From", "Von"),
            &en_de(
                "Where the extension is measured from, in words.",
                "Von wo die Verlängerung gemessen wird, in Worten.",
            ),
        ),
        field(
            "place",
            TEXT,
            &en_de("Place", "Ort"),
            &en_de(
                "Beyond which end, in words.",
                "Über welches Ende hinaus, in Worten.",
            ),
        ),
        field(
            "reach",
            LENGTH,
            &en_de("Extension", "Verlängerung"),
            &en_de(
                "How far it reaches beyond the end; `null` where it runs along another part \
                 only, undecided where not known.",
                "Wie weit er über das Ende hinausreicht; `null`, wo er nur an einem anderen \
                 Teil verläuft, unentschieden, wo unbekannt.",
            ),
        ),
        field(
            "rise",
            LENGTH,
            &en_de("Rise over the extension", "Anstieg über die Verlängerung"),
            &en_de(
                "How far its top rises or falls over `level_over` beyond the end.",
                "Wie weit seine Oberkante über `level_over` hinter dem Ende steigt oder fällt.",
            ),
        ),
        field(
            "over_middle",
            TRUTH,
            &en_de("Over the middle", "Über der Mitte"),
            &en_de(
                "Whether the rail runs along no side, so it may be one piece of a longer rail.",
                "Ob der Handlauf an keiner Seite verläuft und so ein Stück eines längeren sein \
                 kann.",
            ),
        ),
        SCALE,
    ],
};

/// `rail_gaps`.
pub(super) const RAIL_GAPS: MemberDescriptor = MemberDescriptor {
    list: list(
        "rail_gaps",
        RAILS,
        FLIGHTS,
        RAIL_UNMEASURED,
        &en_de("Handrail gaps", "Handlauflücken"),
        &en_de(
            "The gap in plan between consecutive pieces of the handrail along each side.",
            "Die Lücke im Grundriss zwischen aufeinanderfolgenden Stücken des Handlaufs an \
             jeder Seite.",
        ),
    ),
    fields: &[
        STRETCH,
        LABEL,
        field(
            "pair",
            TEXT,
            &en_de("Pieces", "Stücke"),
            &en_de(
                "The two pieces and the side, in words; empty for a side whose pieces cannot \
                 be put in order.",
                "Die beiden Stücke und die Seite, in Worten; leer für eine Seite, deren Stücke \
                 sich nicht ordnen lassen.",
            ),
        ),
        field(
            "rails",
            OBJECTS,
            &en_de("Rails", "Handläufe"),
            &en_de("The two pieces.", "Die beiden Stücke."),
        ),
        field(
            "gap",
            LENGTH,
            &en_de("Gap", "Lücke"),
            &en_de(
                "The gap between them in plan; undecided where not measured.",
                "Die Lücke zwischen ihnen im Grundriss; unentschieden, wo nicht gemessen.",
            ),
        ),
        SCALE,
    ],
};

/// `rail_continuity`.
pub(super) const RAIL_CONTINUITY: MemberDescriptor = MemberDescriptor {
    list: list(
        "rail_continuity",
        &[
            OF,
            objects(
                "rails",
                true,
                &en_de(
                    "The objects that may be handrails or join them.",
                    "Die Objekte, die Handläufe sein oder sie verbinden können.",
                ),
            ),
            length(
                "reach_across",
                true,
                &en_de(
                    "How far outside a run's sides a rail may run.",
                    "Wie weit außerhalb der Seiten eines Laufs ein Handlauf liegen darf.",
                ),
            ),
            length(
                "reach_above",
                true,
                &en_de(
                    "How far above the pitch line a rail may run.",
                    "Wie weit über der Steigungslinie ein Handlauf liegen darf.",
                ),
            ),
            length(
                "level_over",
                false,
                &en_de(
                    "The extension over which a rail's rise is measured, as the handrails are \
                     measured; without it, none.",
                    "Die Verlängerung, über die der Anstieg eines Handlaufs gemessen wird, wie \
                     die Handläufe gemessen werden; ohne Angabe keine.",
                ),
            ),
            length(
                "tolerance",
                false,
                &en_de(
                    "The largest gap between consecutive rails across a landing.",
                    "Die größte Lücke zwischen aufeinanderfolgenden Handläufen über ein Podest.",
                ),
            ),
            length(
                "gap",
                false,
                &en_de(
                    "Without `tolerance`, the largest gap; without either, none.",
                    "Ohne `tolerance` die größte Lücke; ohne beide keine.",
                ),
            ),
        ],
        &["walking-surface", "proximity"],
        RAIL_UNMEASURED,
        &en_de(
            "Handrail breaks across landings",
            "Handlaufunterbrechungen über Podeste",
        ),
        &en_de(
            "Whether the handrail along each side of a ramp stops at a landing between \
             consecutive runs: one search per side found broken or undecided.",
            "Ob der Handlauf an einer Seite einer Rampe an einem Podest zwischen \
             aufeinanderfolgenden Läufen endet: eine Suche je unterbrochen oder \
             unentschieden gefundener Seite.",
        ),
    ),
    fields: SEARCH,
};

/// `rail_obstructions`.
pub(super) const RAIL_OBSTRUCTIONS: MemberDescriptor = MemberDescriptor {
    list: list(
        "rail_obstructions",
        &[
            OF,
            objects(
                "rails",
                true,
                &en_de(
                    "The objects that may be handrails or join them.",
                    "Die Objekte, die Handläufe sein oder sie verbinden können.",
                ),
            ),
            length(
                "level_over",
                false,
                &en_de(
                    "The extension over which a rail's rise is measured, as the handrails are \
                     measured; without it, none.",
                    "Die Verlängerung, über die der Anstieg eines Handlaufs gemessen wird, wie \
                     die Handläufe gemessen werden; ohne Angabe keine.",
                ),
            ),
            objects(
                "surfaces",
                true,
                &en_de(
                    "The accessible surfaces no rail may reach over.",
                    "Die zugänglichen Flächen, über die kein Handlauf reichen darf.",
                ),
            ),
            length(
                "reach_across",
                true,
                &en_de(
                    "How far outside a run's sides a rail may run.",
                    "Wie weit außerhalb der Seiten eines Laufs ein Handlauf liegen darf.",
                ),
            ),
            length(
                "reach_above",
                true,
                &en_de(
                    "How far above the pitch line a rail may run.",
                    "Wie weit über der Steigungslinie ein Handlauf liegen darf.",
                ),
            ),
        ],
        &["walking-surface", "proximity"],
        RAIL_UNMEASURED,
        &en_de(
            "Rails over accessible surfaces",
            "Handläufe über zugänglichen Flächen",
        ),
        &en_de(
            "Whether a ramp's rails reach over a selected accessible surface in plan: one \
             search per rail and surface found over it or undecided, or one undecided where \
             a selection may add one.",
            "Ob die Handläufe einer Rampe im Grundriss über eine ausgewählte zugängliche \
             Fläche reichen: eine Suche je darüber oder unentschieden gefundenem Paar, oder \
             eine unentschiedene, wo eine Auswahl eines hinzufügen kann.",
        ),
    ),
    fields: SEARCH,
};
