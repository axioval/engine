//! A report with findings, a not-evaluated outcome and two tables written
//! as a workbook and read back from its archive.
#![allow(missing_docs)]

use std::io::{Cursor, Read as _};

use axioval_ir::{
    ExternalId, Finding, NotEvaluated, NotEvaluatedReason, Object, ObjectId, Project,
    QuantityDimension, Report, ReportColumn, ReportTable, ReportValue, RuleId, Scope, Severity,
    SourceId,
};
use axioval_xlsx::{Options, export, sheet_names};

fn wall(n: u32) -> ObjectId {
    ObjectId::new(SourceId::new("ifc", "model.ifc").unwrap(), format!("#{n}")).unwrap()
}

fn project() -> Project {
    Project::new(vec![
        Object::new(wall(1), "IfcWall")
            .with_external_id(ExternalId::new("ifc-globalid", "0000000000000000000001").unwrap()),
        Object::new(wall(2), "IfcWall"),
    ])
    .unwrap()
}

/// `takeoff` of rule `walls`: an exact count, a bounded area and an
/// unknown area; `levels` of rule `storeys`: one text column, unknown.
fn report() -> Report {
    let rule = RuleId::new("walls").unwrap();
    let mut takeoff = ReportTable::grouped(
        rule.clone(),
        "takeoff",
        vec!["storey".to_owned()],
        vec![
            ReportColumn::number("count"),
            ReportColumn::quantity("sum_area", QuantityDimension::Area),
        ],
    )
    .unwrap();
    takeoff
        .push_group_row(
            Scope::Project,
            vec!["EG".to_owned()],
            vec![
                ReportValue::exact(2.0),
                ReportValue::measured(2.999_999, 3.000_001),
            ],
        )
        .unwrap();
    takeoff
        .push_group_row(
            Scope::Project,
            vec!["OG".to_owned()],
            vec![ReportValue::exact(1.0), ReportValue::Unknown],
        )
        .unwrap();
    let mut levels = ReportTable::new(
        RuleId::new("storeys").unwrap(),
        "levels",
        vec![ReportColumn::text("name")],
    )
    .unwrap();
    levels
        .push_row(wall(1), vec![ReportValue::Unknown])
        .unwrap();
    levels
        .push_row(wall(2), vec![ReportValue::text("W2")])
        .unwrap();
    Report {
        findings: vec![
            Finding::new(rule.clone(), wall(1), Severity::Error, "too thin")
                .with_related([wall(2)]),
        ],
        not_evaluated: vec![NotEvaluated {
            explanation: None,
            rule_id: rule,
            scope: wall(2).into(),
            reason: NotEvaluatedReason::MissingService,
            message: "no geometry".to_owned(),
            location: None,
        }],
        tables: vec![takeoff, levels],
        ..Report::default()
    }
}

fn options() -> Options {
    Options {
        external_id_scheme: Some("ifc-globalid".to_owned()),
        ..Options::new(1_790_416_800)
    }
}

fn entry(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

/// The `<c>` element of `cell` in `sheet`.
fn cell<'a>(sheet: &'a str, cell: &str) -> &'a str {
    let start = sheet
        .find(&format!("<c r=\"{cell}\""))
        .unwrap_or_else(|| panic!("no cell {cell} in {sheet}"));
    let rest = &sheet[start..];
    let open = rest.find('>').unwrap();
    if rest[..open].ends_with('/') {
        return &rest[..=open];
    }
    &rest[..rest.find("</c>").unwrap() + 4]
}

#[test]
fn two_tables_write_three_sheets_with_numeric_cells() {
    let bytes = export(&report(), &project(), &options()).unwrap();
    let workbook = entry(&bytes, "xl/workbook.xml");
    for name in ["Findings", "takeoff", "levels"] {
        assert!(workbook.contains(&format!("name=\"{name}\"")), "{workbook}");
    }
    assert_eq!(workbook.matches("<sheet ").count(), 3, "{workbook}");

    let strings = entry(&bytes, "xl/sharedStrings.xml");
    for text in [
        "sum_area lower [m²]",
        "sum_area upper [m²]",
        "sum_area exactness",
        "count lower",
        "bounded",
        "exact",
        "not evaluated",
        "0000000000000000000001",
        "missing_service",
    ] {
        assert!(strings.contains(text), "{text}: {strings}");
    }

    // Row 3 is EG: scope, kind, GlobalId, storey, then count and area.
    let takeoff = entry(&bytes, "xl/worksheets/sheet2.xml");
    assert_eq!(cell(&takeoff, "E3"), "<c r=\"E3\"><v>2</v></c>");
    assert_eq!(cell(&takeoff, "F3"), "<c r=\"F3\"><v>2</v></c>");
    // The interval keeps both bounds as numbers.
    assert_eq!(cell(&takeoff, "H3"), "<c r=\"H3\"><v>2.999999</v></c>");
    assert_eq!(cell(&takeoff, "I3"), "<c r=\"I3\"><v>3.000001</v></c>");
    // The unknown area is two shaded blanks and a `not evaluated` marker.
    assert!(!cell(&takeoff, "H4").contains("<v>"), "{takeoff}");
    assert!(cell(&takeoff, "H4").contains(" s=\""), "{takeoff}");
    assert!(cell(&takeoff, "J4").contains("t=\"s\""), "{takeoff}");

    // An unknown text value is a shaded blank, not an empty string.
    let levels = entry(&bytes, "xl/worksheets/sheet3.xml");
    assert!(!cell(&levels, "D3").contains("<v>"), "{levels}");
    assert!(cell(&levels, "D3").contains(" s=\""), "{levels}");
    assert!(cell(&levels, "D4").contains("t=\"s\""), "{levels}");

    let findings = entry(&bytes, "xl/worksheets/sheet1.xml");
    assert!(findings.contains("<c r=\"A3\""), "{findings}");
}

#[test]
fn the_same_report_writes_the_same_bytes() {
    let first = export(&report(), &project(), &options()).unwrap();
    let second = export(&report(), &project(), &options()).unwrap();
    assert_eq!(first, second);
    let core = entry(&first, "docProps/core.xml");
    assert!(core.contains("2026-09-26T"), "{core}");
}

#[test]
fn tables_sharing_a_name_are_told_apart_by_rule() {
    let table = |rule: &str| {
        ReportTable::new(
            RuleId::new(rule).unwrap(),
            "takeoff",
            vec![ReportColumn::number("count")],
        )
        .unwrap()
    };
    let long = "a".repeat(40);
    let names = sheet_names(&[
        table("walls"),
        table("doors"),
        ReportTable::new(
            RuleId::new(&long).unwrap(),
            "findings",
            vec![ReportColumn::number("count")],
        )
        .unwrap(),
    ]);
    assert_eq!(names, ["walls takeoff", "doors takeoff", "findings (2)"]);
}
