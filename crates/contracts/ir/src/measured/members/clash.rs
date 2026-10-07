//! The member lists `clash` and `clash-matrix` judge: the candidate pairs
//! a rule's selected objects form with their counterparts, each measured
//! once (separation, penetration, containment, overlap extents, shared
//! volume) beside the tolerances it is judged against, or left open with
//! why.

use super::super::registry::en_de;
use super::super::{
    LocalizedText, MeasuredDescriptor, MeasuredExactness, MeasuredParameter, MeasuredParameterKind,
    MeasuredSubject,
};
use super::{LENGTH, MemberDescriptor, MemberField, MemberFieldKind, TRUTH, field};
use crate::QuantityDimension;

const TEXT: MemberFieldKind = MemberFieldKind::Text;
const OBJECTS: MemberFieldKind = MemberFieldKind::Objects;
const VOLUME: MemberFieldKind = MemberFieldKind::Number {
    dimension: Some(QuantityDimension::Volume),
};

const fn parameter(
    key: &'static str,
    kind: MeasuredParameterKind,
    required: bool,
    help: &'static [LocalizedText],
) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind,
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
    parameter(
        key,
        MeasuredParameterKind::Length { minimum: 0.0 },
        required,
        help,
    )
}

const fn truth(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    parameter(key, MeasuredParameterKind::Truth, false, help)
}

const fn table(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    parameter(key, MeasuredParameterKind::Table, false, help)
}

const SUBJECTS: MeasuredParameter = parameter(
    "subjects",
    MeasuredParameterKind::Objects,
    true,
    &en_de(
        "The objects checked: their source kinds, or `@` a selector parameter of the rule \
         (`@selection`, in a template, the rule's own selection); only those surely picked \
         form pairs.",
        "Die geprüften Objekte: ihre Quellarten oder mit `@` ein Selektorparameter der Regel \
         (`@selection`, in einer Vorlage, die eigene Auswahl der Regel); nur sicher getroffene \
         bilden Paare.",
    ),
);

const COUNTERPARTS: MeasuredParameter = parameter(
    "counterparts",
    MeasuredParameterKind::Objects,
    true,
    &en_de(
        "The objects they are checked against, named as `subjects` are.",
        "Die Objekte, gegen die sie geprüft werden, benannt wie bei `subjects`.",
    ),
);

/// The parameters both lists read beside their tolerances: exclusions,
/// grouping keys, duplicates, grades and tolerance cases, each a rule's
/// parameter of the same name.
const SHARED: [MeasuredParameter; 13] = [
    parameter(
        "exclude_paths",
        MeasuredParameterKind::Paths,
        false,
        &en_de(
            "Relationship paths: a pair whose objects reach a common target along one, or \
             one the other, is left out.",
            "Beziehungspfade: ein Paar, dessen Objekte entlang eines davon ein gemeinsames \
             Ziel oder einander erreichen, bleibt außen vor.",
        ),
    ),
    parameter(
        "exclude_target_property",
        MeasuredParameterKind::Property,
        false,
        &en_de(
            "Targets reached along an exclusion path stating the same value of this property \
             leave the pair out as well.",
            "Entlang eines Ausschlusspfads erreichte Ziele mit demselben Wert dieser \
             Eigenschaft lassen das Paar ebenfalls außen vor.",
        ),
    ),
    truth(
        "exclude_same_layer",
        &en_de(
            "Whether a pair on a shared presentation layer of one source is left out.",
            "Ob ein Paar auf einer gemeinsamen Darstellungsebene einer Quelle außen vor bleibt.",
        ),
    ),
    parameter(
        "group_by",
        MeasuredParameterKind::Choice {
            options: &["type_pair", "subject", "similar"],
        },
        false,
        &en_de(
            "How reported pairs are keyed into groups (`group`): by the pair of types, by \
             subject, or as similar pairs whose extents round alike.",
            "Wie gemeldete Paare zu Gruppen verschlüsselt werden (`group`): nach dem Typpaar, \
             nach Subjekt oder als ähnliche Paare mit gleich gerundeten Ausdehnungen.",
        ),
    ),
    truth(
        "per_storey",
        &en_de(
            "Whether the storeys the pair reaches along `storey_path` are part of its key.",
            "Ob die Geschosse, die das Paar über `storey_path` erreicht, zu seinem Schlüssel \
             gehören.",
        ),
    ),
    parameter(
        "storey_path",
        MeasuredParameterKind::Text,
        false,
        &en_de(
            "The relationship path from an object to its storey, its steps separated by \
             spaces.",
            "Der Beziehungspfad von einem Objekt zu seinem Geschoss, die Schritte durch \
             Leerzeichen getrennt.",
        ),
    ),
    parameter(
        "group_property",
        MeasuredParameterKind::Property,
        false,
        &en_de(
            "A property whose values similar pairs must share.",
            "Eine Eigenschaft, deren Werte ähnliche Paare teilen müssen.",
        ),
    ),
    length(
        "group_tolerance_metres",
        false,
        &en_de(
            "The rounding step of similar pairs' extents, in metres.",
            "Der Rundungsschritt der Ausdehnungen ähnlicher Paare, in Metern.",
        ),
    ),
    table(
        "severity_by_class",
        &en_de(
            "The severity per class, as the rule states it: read by the template's grading, \
             checked with the pairs.",
            "Der Schweregrad je Klasse, wie die Regel ihn angibt: von der Einstufung der \
             Vorlage gelesen, mit den Paaren geprüft.",
        ),
    ),
    parameter(
        "grade_by",
        MeasuredParameterKind::Choice {
            options: &["smallest_extent", "volume"],
        },
        false,
        &en_de(
            "What intersections are graded by: read by the template's grading, checked with \
             the pairs.",
            "Wonach Durchdringungen eingestuft werden: von der Einstufung der Vorlage \
             gelesen, mit den Paaren geprüft.",
        ),
    ),
    table(
        "severity_grades",
        &en_de(
            "The grades of intersections: read by the template's grading, checked with the \
             pairs.",
            "Die Stufen der Durchdringungen: von der Einstufung der Vorlage gelesen, mit den \
             Paaren geprüft.",
        ),
    ),
    table(
        "duplicate_quantities",
        &en_de(
            "The properties duplicates are compared by (`copies`).",
            "Die Eigenschaften, nach denen Duplikate verglichen werden (`copies`).",
        ),
    ),
    table(
        "tolerance_cases",
        &en_de(
            "Tolerances along the elements' own axes that may excuse an intersection \
             (`excused`).",
            "Toleranzen entlang der eigenen Achsen der Elemente, die eine Durchdringung \
             entschuldigen können (`excused`).",
        ),
    ),
];

/// The two selections, `own` and the shared parameters, in that order.
const fn parameters<const N: usize, const M: usize>(
    own: [MeasuredParameter; M],
) -> [MeasuredParameter; N] {
    let mut parameters = [COUNTERPARTS; N];
    parameters[1] = SUBJECTS;
    let mut index = 0;
    while index < M {
        parameters[2 + index] = own[index];
        index += 1;
    }
    let mut shared = 0;
    while shared < SHARED.len() {
        parameters[2 + M + shared] = SHARED[shared];
        shared += 1;
    }
    parameters
}

const fn text_field(
    name: &'static str,
    label: &'static [LocalizedText],
    help: &'static [LocalizedText],
) -> MemberField {
    field(name, TEXT, label, help)
}

/// The fields of every pair, and of every object left open.
const FIELDS: &[MemberField] = &[
    field(
        "subject",
        OBJECTS,
        &en_de("Subject", "Subjekt"),
        &en_de(
            "The selected object of the pair, or the object an open item is about.",
            "Das ausgewählte Objekt des Paars oder das Objekt eines offenen Eintrags.",
        ),
    ),
    field(
        "counterpart",
        OBJECTS,
        &en_de("Counterpart", "Gegenstück"),
        &en_de(
            "The counterpart of the pair; none for an open item about one object.",
            "Das Gegenstück des Paars; keines bei einem offenen Eintrag zu einem Objekt.",
        ),
    ),
    text_field(
        "open",
        &en_de("Open", "Offen"),
        &en_de(
            "Why the item is no pair to judge (a body that cannot be measured, a selection \
             or cell that cannot be decided); empty for a pair.",
            "Warum der Eintrag kein zu beurteilendes Paar ist (ein nicht messbarer Körper, \
             eine nicht entscheidbare Auswahl oder Zelle); leer bei einem Paar.",
        ),
    ),
    text_field(
        "reason",
        &en_de("Reason", "Grund"),
        &en_de(
            "The not-evaluated reason of an open item, as reports spell it.",
            "Der Grund eines offenen Eintrags, wie Berichte ihn schreiben.",
        ),
    ),
    text_field(
        "excluded",
        &en_de("May be excluded", "Möglicherweise ausgeschlossen"),
        &en_de(
            "Why the pair may or may not be excluded, where an exclusion cannot be decided; \
             empty where it is surely not excluded. A surely excluded pair is not listed.",
            "Warum das Paar ausgeschlossen sein kann, wo ein Ausschluss nicht entscheidbar \
             ist; leer, wo es sicher nicht ausgeschlossen ist. Ein sicher ausgeschlossenes \
             Paar wird nicht aufgeführt.",
        ),
    ),
    text_field(
        "excluded_reason",
        &en_de("Exclusion reason", "Ausschlussgrund"),
        &en_de(
            "The not-evaluated reason of an undecided exclusion, as reports spell it.",
            "Der Grund eines nicht entscheidbaren Ausschlusses, wie Berichte ihn schreiben.",
        ),
    ),
    field(
        "separation",
        LENGTH,
        &en_de("Separation", "Abstand"),
        &en_de(
            "The separation the proximity service measured, in metres; zero where the \
             bodies meet or overlap.",
            "Der vom Näheservice gemessene Abstand in Metern; null, wo sich die Körper \
             berühren oder überlappen.",
        ),
    ),
    field(
        "certified",
        LENGTH,
        &en_de("Certified separation", "Zertifizierter Abstand"),
        &en_de(
            "The separation certified on exact boundaries, an interval; none where not \
             certified.",
            "Der auf exakten Rändern zertifizierte Abstand, ein Intervall; keiner, wo nicht \
             zertifiziert.",
        ),
    ),
    field(
        "penetration",
        LENGTH,
        &en_de("Penetration", "Eindringtiefe"),
        &en_de(
            "How deep one body surely reaches into the other; none where neither is a closed \
             solid.",
            "Wie tief ein Körper sicher in den anderen reicht; keine, wo keiner ein \
             geschlossener Volumenkörper ist.",
        ),
    ),
    field(
        "inside",
        TRUTH,
        &en_de("Inside", "Innen"),
        &en_de(
            "Whether the subject lies wholly inside the counterpart.",
            "Ob das Subjekt ganz im Gegenstück liegt.",
        ),
    ),
    field(
        "contains",
        TRUTH,
        &en_de("Contains", "Enthält"),
        &en_de(
            "Whether the subject wholly contains the counterpart.",
            "Ob das Subjekt das Gegenstück ganz enthält.",
        ),
    ),
    field(
        "hausdorff",
        LENGTH,
        &en_de("Surface distance", "Flächenabstand"),
        &en_de(
            "The Hausdorff distance between the surfaces, at least the separation; without \
             an upper end where not measured.",
            "Der Hausdorff-Abstand der Oberflächen, mindestens der Abstand; ohne oberes \
             Ende, wo nicht gemessen.",
        ),
    ),
    text_field(
        "hausdorff_upper",
        &en_de("Surface distance, upper", "Flächenabstand, oben"),
        &en_de(
            "The Hausdorff distance's upper end in words (`0.0100 m`, or `an unmeasured \
             distance`).",
            "Das obere Ende des Hausdorff-Abstands in Worten (`0.0100 m` oder `an \
             unmeasured distance`).",
        ),
    ),
    field(
        "horizontal",
        LENGTH,
        &en_de("Extent in plan", "Ausdehnung im Grundriss"),
        &en_de(
            "The intersection's lesser extent along the plan axes; undecided where not \
             measured.",
            "Die kleinere Ausdehnung der Durchdringung entlang der Grundrissachsen; \
             unentschieden, wo nicht gemessen.",
        ),
    ),
    field(
        "vertical",
        LENGTH,
        &en_de("Extent in height", "Ausdehnung in der Höhe"),
        &en_de(
            "The intersection's extent in height; undecided where not measured.",
            "Die Ausdehnung der Durchdringung in der Höhe; unentschieden, wo nicht gemessen.",
        ),
    ),
    field(
        "smallest_extent",
        LENGTH,
        &en_de("Smallest extent", "Kleinste Ausdehnung"),
        &en_de(
            "The intersection's least extent along x, y and z; undecided where not measured.",
            "Die kleinste Ausdehnung der Durchdringung entlang x, y und z; unentschieden, wo \
             nicht gemessen.",
        ),
    ),
    field(
        "shared_volume",
        VOLUME,
        &en_de("Shared volume", "Gemeinsames Volumen"),
        &en_de(
            "The certified volume the bodies share; undecided where not measured.",
            "Das zertifizierte gemeinsame Volumen der Körper; unentschieden, wo nicht \
             gemessen.",
        ),
    ),
    field(
        "penetration_tolerance",
        LENGTH,
        &en_de("Penetration tolerance", "Eindringtoleranz"),
        &en_de(
            "The penetration the pair is allowed: the rule's, or its cell's.",
            "Die dem Paar erlaubte Eindringtiefe: die der Regel oder seiner Zelle.",
        ),
    ),
    field(
        "clearance",
        LENGTH,
        &en_de("Clearance", "Freiraum"),
        &en_de(
            "The clearance the pair must keep; none where none is declared.",
            "Der einzuhaltende Freiraum; keiner, wo keiner angegeben ist.",
        ),
    ),
    field(
        "duplicate_tolerance",
        LENGTH,
        &en_de("Duplicate tolerance", "Duplikattoleranz"),
        &en_de(
            "How far apart duplicate surfaces may lie; zero unless declared.",
            "Wie weit Oberflächen von Duplikaten auseinanderliegen dürfen; null ohne Angabe.",
        ),
    ),
    field(
        "horizontal_tolerance",
        LENGTH,
        &en_de("Plan tolerance", "Grundrisstoleranz"),
        &en_de(
            "The extent in plan an intersection must exceed; zero unless declared.",
            "Die Ausdehnung im Grundriss, die eine Durchdringung überschreiten muss; null \
             ohne Angabe.",
        ),
    ),
    field(
        "vertical_tolerance",
        LENGTH,
        &en_de("Height tolerance", "Höhentoleranz"),
        &en_de(
            "The extent in height an intersection must exceed; zero unless declared.",
            "Die Ausdehnung in der Höhe, die eine Durchdringung überschreiten muss; null \
             ohne Angabe.",
        ),
    ),
    field(
        "volume_tolerance",
        VOLUME,
        &en_de("Volume tolerance", "Volumentoleranz"),
        &en_de(
            "The shared volume an intersection must exceed; zero unless declared.",
            "Das gemeinsame Volumen, das eine Durchdringung überschreiten muss; null ohne \
             Angabe.",
        ),
    ),
    field(
        "report_duplicates",
        TRUTH,
        &en_de("Duplicates reported", "Duplikate gemeldet"),
        &en_de(
            "Whether duplicates are reported; true unless switched off.",
            "Ob Duplikate gemeldet werden; wahr, wenn nicht abgeschaltet.",
        ),
    ),
    field(
        "report_containment",
        TRUTH,
        &en_de("Containment reported", "Einschluss gemeldet"),
        &en_de(
            "Whether contained bodies are reported; true unless switched off.",
            "Ob eingeschlossene Körper gemeldet werden; wahr, wenn nicht abgeschaltet.",
        ),
    ),
    field(
        "report_intersections",
        TRUTH,
        &en_de("Intersections reported", "Durchdringungen gemeldet"),
        &en_de(
            "Whether intersections are reported; true unless switched off.",
            "Ob Durchdringungen gemeldet werden; wahr, wenn nicht abgeschaltet.",
        ),
    ),
    field(
        "excused",
        TRUTH,
        &en_de("Excused", "Entschuldigt"),
        &en_de(
            "Whether a tolerance case excuses the pair's intersection along the elements' own \
             axes; undecided where a filter, a frame or an extent cannot decide it.",
            "Ob ein Toleranzfall die Durchdringung des Paars entlang der eigenen Achsen der \
             Elemente entschuldigt; unentschieden, wo ein Filter, ein Achsensystem oder eine \
             Ausdehnung es nicht entscheidet.",
        ),
    ),
    text_field(
        "case",
        &en_de("Tolerance cases", "Toleranzfälle"),
        &en_de(
            "What the tolerance cases that may apply found, in words; empty where none \
             applies.",
            "Was die möglicherweise geltenden Toleranzfälle ergaben, in Worten; leer, wo \
             keiner gilt.",
        ),
    ),
    text_field(
        "note",
        &en_de("Fidelity", "Genauigkeit"),
        &en_de(
            "Words marking a measurement on tessellated geometry as approximate; empty for \
             exact geometry.",
            "Worte, die eine Messung an tessellierter Geometrie als näherungsweise \
             kennzeichnen; leer bei exakter Geometrie.",
        ),
    ),
    text_field(
        "reach",
        &en_de("Reach", "Reichweite"),
        &en_de(
            "The intersection's extents and shared volume in words, where its tolerances \
             declare them.",
            "Ausdehnungen und gemeinsames Volumen der Durchdringung in Worten, wo ihre \
             Toleranzen sie angeben.",
        ),
    ),
    text_field(
        "copies",
        &en_de("Copies", "Kopien"),
        &en_de(
            "What two duplicates differ or agree in (type, volume, `duplicate_quantities`), \
             in words.",
            "Worin zwei Duplikate sich unterscheiden oder übereinstimmen (Typ, Volumen, \
             `duplicate_quantities`), in Worten.",
        ),
    ),
    field(
        "group",
        TEXT,
        &en_de("Group", "Gruppe"),
        &en_de(
            "The key grouping the pair's finding under `group_by`; undecided, with why, where \
             it cannot be read.",
            "Der Schlüssel, der den Befund des Paars nach `group_by` gruppiert; \
             unentschieden, mit Grund, wo er nicht lesbar ist.",
        ),
    ),
    text_field(
        "sides",
        &en_de("Types", "Typen"),
        &en_de(
            "The pair's two types (and `group_property` values), in order: `A with B`.",
            "Die beiden Typen des Paars (und Werte von `group_property`), geordnet: `A with \
             B`.",
        ),
    ),
    text_field(
        "on",
        &en_de("Storeys", "Geschosse"),
        &en_de(
            "The storeys the pair reaches, in words (` on s1, s2`), with `per_storey`.",
            "Die Geschosse, die das Paar erreicht, in Worten (` on s1, s2`), mit \
             `per_storey`.",
        ),
    ),
];

/// The fields only the matrix's pairs state: the cell judging them.
const MATRIX_EXTRA: [MemberField; 5] = [
    field(
        "cell",
        TEXT,
        &en_de("Cell", "Zelle"),
        &en_de(
            "The cell judging the pair, in words (` (clash matrix cell 0 `walls`)`); empty for \
             a pair no cell covers.",
            "Die Zelle, die das Paar beurteilt, in Worten (` (clash matrix cell 0 `walls`)`); \
             leer bei einem Paar ohne Zelle.",
        ),
    ),
    field(
        "severity",
        TEXT,
        &en_de("Cell severity", "Schweregrad der Zelle"),
        &en_de(
            "The severity the cell states; empty where it states none.",
            "Der Schweregrad, den die Zelle angibt; leer, wo sie keinen angibt.",
        ),
    ),
    field(
        "unmatched",
        TRUTH,
        &en_de("Unmatched", "Ohne Zelle"),
        &en_de(
            "Whether no cell covers the pair (listed with `report_unmatched` only).",
            "Ob keine Zelle das Paar abdeckt (nur mit `report_unmatched` aufgeführt).",
        ),
    ),
    field(
        "subject_category",
        TEXT,
        &en_de("Subject's categories", "Kategorien des Subjekts"),
        &en_de(
            "The subject and the categories the cells test, in words.",
            "Das Subjekt und die von den Zellen geprüften Kategorien, in Worten.",
        ),
    ),
    field(
        "counterpart_category",
        TEXT,
        &en_de("Counterpart's categories", "Kategorien des Gegenstücks"),
        &en_de(
            "The counterpart and the categories the cells test, in words.",
            "Das Gegenstück und die von den Zellen geprüften Kategorien, in Worten.",
        ),
    ),
];

/// The fields of the matrix's pairs: every pair's, and the cell judging it.
const MATRIX_FIELDS: &[MemberField] = &{
    let mut fields = [FIELDS[0]; FIELDS.len() + MATRIX_EXTRA.len()];
    let mut index = 0;
    while index < fields.len() {
        fields[index] = if index < FIELDS.len() {
            FIELDS[index]
        } else {
            MATRIX_EXTRA[index - FIELDS.len()]
        };
        index += 1;
    }
    fields
};

const CLASH_PARAMETERS: &[MeasuredParameter] = &parameters::<24, 9>([
    length(
        "penetration_tolerance_metres",
        true,
        &en_de(
            "How deep a body may reach into another before it intersects it, in metres.",
            "Wie tief ein Körper in einen anderen reichen darf, bevor er ihn durchdringt, \
                 in Metern.",
        ),
    ),
    length(
        "clearance_metres",
        false,
        &en_de(
            "The clearance pairs must keep, in metres; none without it.",
            "Der Freiraum, den Paare einhalten müssen, in Metern; ohne Angabe keiner.",
        ),
    ),
    length(
        "duplicate_tolerance_metres",
        false,
        &en_de(
            "How far apart duplicate surfaces may lie, in metres; zero without it.",
            "Wie weit Oberflächen von Duplikaten auseinanderliegen dürfen, in Metern; ohne \
                 Angabe null.",
        ),
    ),
    length(
        "horizontal_tolerance_metres",
        false,
        &en_de(
            "The extent in plan an intersection must exceed, in metres; zero without it.",
            "Die Ausdehnung im Grundriss, die eine Durchdringung überschreiten muss, in \
                 Metern; ohne Angabe null.",
        ),
    ),
    length(
        "vertical_tolerance_metres",
        false,
        &en_de(
            "The extent in height an intersection must exceed, in metres; zero without it.",
            "Die Ausdehnung in der Höhe, die eine Durchdringung überschreiten muss, in \
                 Metern; ohne Angabe null.",
        ),
    ),
    parameter(
        "volume_tolerance_cubic_metres",
        MeasuredParameterKind::Number { minimum: 0.0 },
        false,
        &en_de(
            "The shared volume an intersection must exceed, in cubic metres; zero without \
                 it.",
            "Das gemeinsame Volumen, das eine Durchdringung überschreiten muss, in \
                 Kubikmetern; ohne Angabe null.",
        ),
    ),
    truth(
        "report_duplicates",
        &en_de(
            "Whether duplicates are reported; true without it.",
            "Ob Duplikate gemeldet werden; ohne Angabe wahr.",
        ),
    ),
    truth(
        "report_containment",
        &en_de(
            "Whether contained bodies are reported; true without it.",
            "Ob eingeschlossene Körper gemeldet werden; ohne Angabe wahr.",
        ),
    ),
    truth(
        "report_intersections",
        &en_de(
            "Whether intersections are reported; true without it.",
            "Ob Durchdringungen gemeldet werden; ohne Angabe wahr.",
        ),
    ),
]);

const MATRIX_PARAMETERS: &[MeasuredParameter] = &parameters::<24, 9>([
    table(
        "cells",
        &en_de(
            "The matrix's cells, as the rule states them: each keys both sides of a pair \
                 and states its tolerances and severity.",
            "Die Zellen der Matrix, wie die Regel sie angibt: jede verschlüsselt beide \
                 Seiten eines Paars und gibt seine Toleranzen und seinen Schweregrad an.",
        ),
    ),
    parameter(
        "key_1",
        MeasuredParameterKind::Property,
        false,
        &en_de(
            "The property a cell's first key patterns test.",
            "Die Eigenschaft, die die ersten Schlüsselmuster einer Zelle prüfen.",
        ),
    ),
    parameter(
        "key_2",
        MeasuredParameterKind::Property,
        false,
        &en_de(
            "The property a cell's second key patterns test.",
            "Die Eigenschaft, die die zweiten Schlüsselmuster einer Zelle prüfen.",
        ),
    ),
    parameter(
        "key_3",
        MeasuredParameterKind::Property,
        false,
        &en_de(
            "The property a cell's third key patterns test.",
            "Die Eigenschaft, die die dritten Schlüsselmuster einer Zelle prüfen.",
        ),
    ),
    truth(
        "case_sensitive",
        &en_de(
            "Whether the cells' patterns match case-sensitively; true without it.",
            "Ob die Muster der Zellen Groß- und Kleinschreibung beachten; ohne Angabe \
                 wahr.",
        ),
    ),
    truth(
        "symmetric",
        &en_de(
            "Whether a cell covers a pair either way round; true without it.",
            "Ob eine Zelle ein Paar in beiden Richtungen abdeckt; ohne Angabe wahr.",
        ),
    ),
    truth(
        "report_unmatched",
        &en_de(
            "Whether a pair no cell covers is listed (`unmatched`); false without it.",
            "Ob ein Paar ohne Zelle aufgeführt wird (`unmatched`); ohne Angabe falsch.",
        ),
    ),
    truth(
        "exclude_same_system",
        &en_de(
            "Whether pairs reaching a common system along `system_path` are left out; \
                 true without it.",
            "Ob Paare, die entlang `system_path` ein gemeinsames System erreichen, außen \
                 vor bleiben; ohne Angabe wahr.",
        ),
    ),
    parameter(
        "system_path",
        MeasuredParameterKind::Text,
        false,
        &en_de(
            "The relationship path from an object to its system, its steps separated by \
                 spaces.",
            "Der Beziehungspfad von einem Objekt zu seinem System, die Schritte durch \
                 Leerzeichen getrennt.",
        ),
    ),
]);

const NOT_EVALUATED: &[&str] = &[
    "the declaration is not realisable (a target property without an exclusion path)",
    "the broad phase refuses the objects' extents",
];

pub(super) const CLASH_MATRIX_PAIRS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "clash_matrix_pairs",
        parameters: MATRIX_PARAMETERS,
        dimension: None,
        services: &[
            "proximity",
            "object-frame",
            "property-resolution",
            "relationship-selection",
            "source-disciplines",
        ],
        exactness: MeasuredExactness::Measured,
        not_evaluated: NOT_EVALUATED,
        label: &en_de("Clash matrix pairs", "Paare der Kollisionsmatrix"),
        help: &en_de(
            "The candidate pairs of the subjects and their counterparts, as `clash-matrix` \
             measures them: each pair a cell covers, measured once beside that cell's \
             tolerances, each pair no cell covers where `report_unmatched` lists it, and \
             every object or pair that cannot be measured or whose cell cannot be chosen, \
             open with why.",
            "Die Kandidatenpaare der Subjekte und ihrer Gegenstücke, wie `clash-matrix` sie \
             misst: jedes von einer Zelle abgedeckte Paar, einmal neben den Toleranzen dieser \
             Zelle gemessen, jedes Paar ohne Zelle, wo `report_unmatched` es aufführt, und \
             jedes Objekt oder Paar, das nicht messbar ist oder dessen Zelle nicht wählbar \
             ist, offen mit Grund.",
        ),
        subject: MeasuredSubject::Project,
    },
    fields: MATRIX_FIELDS,
};

pub(super) const CLASH_PAIRS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "clash_pairs",
        parameters: CLASH_PARAMETERS,
        dimension: None,
        services: &[
            "proximity",
            "object-frame",
            "property-resolution",
            "relationship-selection",
        ],
        exactness: MeasuredExactness::Measured,
        not_evaluated: NOT_EVALUATED,
        label: &en_de("Clash pairs", "Kollisionspaare"),
        help: &en_de(
            "The candidate pairs of the subjects and their counterparts the broad phase \
             proposes within the clearance, as `clash` measures them: each pair measured \
             once beside the rule's tolerances, and every object or pair that cannot be \
             measured, open with why. A pair surely excluded is not listed.",
            "Die Kandidatenpaare der Subjekte und ihrer Gegenstücke, die die Grobphase \
             innerhalb des Freiraums vorschlägt, wie `clash` sie misst: jedes Paar einmal \
             neben den Toleranzen der Regel gemessen, und jedes nicht messbare Objekt oder \
             Paar offen mit Grund. Ein sicher ausgeschlossenes Paar wird nicht aufgeführt.",
        ),
        subject: MeasuredSubject::Project,
    },
    fields: FIELDS,
};
