//! A report rendered through the default template and through custom ones.
#![allow(missing_docs)]

use axioval_html::{DEFAULT_TEMPLATE, Options, Slot, Template, TemplateError, render};
use axioval_ir::{
    DateTime, ExternalId, Finding, Location, NotEvaluated, NotEvaluatedReason, Object, ObjectId,
    Place, Project, QuantityDimension, Report, ReportColumn, ReportTable, ReportValue, RuleId,
    Scope, Severity, SourceId,
};

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

/// A categorised, located finding whose message holds markup, an outcome
/// not evaluated, and a takeoff with an exact, a bounded and an unknown
/// value.
fn report() -> Report {
    let rule = RuleId::new("walls").unwrap();
    let mut finding = Finding::new(
        rule.clone(),
        wall(1),
        Severity::Error,
        "thinner than <script>alert(1)</script> & 0.2 m",
    )
    .with_related([wall(2)]);
    finding.categories = vec!["external".to_owned(), "load-bearing".to_owned()];
    finding.location = Some(Location {
        storeys: vec![Place {
            id: wall(9),
            name: Some("EG".to_owned()),
        }],
        ..Location::default()
    });
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
    Report {
        findings: vec![finding],
        not_evaluated: vec![NotEvaluated {
            explanation: None,
            rule_id: RuleId::new("doors").unwrap(),
            scope: wall(2).into(),
            reason: NotEvaluatedReason::MissingService,
            message: "no geometry".to_owned(),
            location: None,
        }],
        tables: vec![takeoff],
        ..Report::default()
    }
}

fn options() -> Options {
    Options {
        external_id_scheme: Some("ifc-globalid".to_owned()),
        ..Options::new("2026-09-26T10:00:00Z")
    }
}

/// Where `needle` first appears in `html`.
fn at(html: &str, needle: &str) -> usize {
    html.find(needle)
        .unwrap_or_else(|| panic!("no {needle:?} in {html}"))
}

#[test]
fn the_default_template_renders_one_self_contained_file() {
    let html = render(&report(), &project(), &Template::default(), &options());
    assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
    for id in [
        "id=\"cover\"",
        "id=\"summary\"",
        "id=\"rules\"",
        "id=\"categories\"",
        "id=\"findings\"",
        "id=\"not-evaluated\"",
        "id=\"stale-decisions\"",
        "id=\"tables\"",
    ] {
        at(&html, id);
    }
    // Sections in the default order.
    assert!(at(&html, "id=\"summary\"") < at(&html, "id=\"findings\""));
    assert!(at(&html, "id=\"findings\"") < at(&html, "id=\"tables\""));
    // Nothing is fetched or run: the style is inline, there is no script.
    for external in ["http://", "https://", "<script", "<link", " src="] {
        assert!(!html.contains(external), "{external}: {html}");
    }
    at(&html, "@page");
    // Report text is escaped.
    at(
        &html,
        "thinner than &lt;script&gt;alert(1)&lt;/script&gt; &amp; 0.2 m",
    );
    // The finding with its object, alias, category, location.
    at(
        &html,
        "<span class=\"kind\">IfcWall</span> <code>0000000000000000000001</code>",
    );
    at(&html, "external / load-bearing");
    at(&html, "Storey: EG");
    at(&html, "Not passed: 1 finding(s)");
    // The interval keeps both bounds; the unknown value says so.
    at(
        &html,
        "<td class=\"number bounded\">2.999999 – 3.000001</td>",
    );
    at(&html, "<td class=\"unknown\">not evaluated</td>");
    at(&html, "sum_area [m²]");
    // The outcome not evaluated, and its reason.
    at(&html, "missing service");
}

#[test]
fn the_same_report_renders_the_same_bytes() {
    let first = render(&report(), &project(), &Template::default(), &options());
    let second = render(&report(), &project(), &Template::default(), &options());
    assert_eq!(first, second);
}

#[test]
fn a_custom_template_reorders_and_leaves_out_sections() {
    let template = Template::parse(
        "<html><head><title>{{ title }}</title><style>{{style}}</style></head><body>\n\
         {{tables}}\n{{not-evaluated}}\n{{summary}}\n</body></html>\n",
    )
    .unwrap();
    assert_eq!(
        template.slots().collect::<Vec<_>>(),
        [
            Slot::Title,
            Slot::Style,
            Slot::Tables,
            Slot::NotEvaluated,
            Slot::Summary
        ]
    );
    let html = render(&report(), &project(), &template, &options());
    assert!(at(&html, "id=\"tables\"") < at(&html, "id=\"not-evaluated\""));
    assert!(at(&html, "id=\"not-evaluated\"") < at(&html, "id=\"summary\""));
    assert!(!html.contains("id=\"findings\""), "{html}");
    at(&html, "<title>Check report</title>");
}

#[test]
fn a_template_hiding_the_outcomes_not_evaluated_is_refused() {
    assert_eq!(
        Template::parse("{{summary}}"),
        Err(TemplateError::Missing(Slot::NotEvaluated))
    );
    assert_eq!(
        Template::parse("{{summary}}\n{{not-evaluated}}\n{{summary}}"),
        Err(TemplateError::Repeated {
            line: 3,
            slot: Slot::Summary
        })
    );
    assert!(matches!(
        Template::parse("{{summary}}{{not-evaluated}}\n{{ script }}"),
        Err(TemplateError::UnknownSlot { line: 2, .. })
    ));
    assert_eq!(
        Template::parse("{{summary\n}}"),
        Err(TemplateError::Unclosed { line: 1 })
    );
    // Inline values may repeat.
    Template::parse("{{title}} {{title}} {{summary}} {{not-evaluated}}").unwrap();
    // The built-in template places every slot.
    let default = Template::parse(DEFAULT_TEMPLATE).unwrap();
    assert_eq!(default.slots().count(), Slot::ALL.len() - 1);
}

#[test]
fn a_clean_run_says_it_passed_and_a_decision_is_shown() {
    let mut report = report();
    report.not_evaluated.clear();
    let html = render(
        &Report::default(),
        &project(),
        &Template::default(),
        &options(),
    );
    at(
        &html,
        "Passed: everything was evaluated and nothing was found",
    );
    let decided = &mut report.findings[0];
    decided.decision = Some(axioval_ir::FindingDecision {
        status: axioval_ir::DecisionStatus::Accepted,
        author: "reviewer".to_owned(),
        date: "2026-09-27T08:00:00Z".parse::<DateTime>().unwrap(),
        comments: Vec::new(),
        assigned_to: Some("structure".to_owned()),
        due_date: None,
        priority: None,
        labels: Vec::new(),
        evidence: axioval_ir::EvidenceCheck::Changed,
        changes: Vec::new(),
    });
    let html = render(&report, &project(), &Template::default(), &options());
    at(&html, "<strong>accepted</strong> by reviewer");
    at(&html, "changed since the decision");
    at(&html, "assigned to structure");
}

#[test]
fn an_expression_rules_finding_shows_its_deciding_path() {
    let step = |path: &str, kind: &str, label: Option<&str>, value: &str, deciding: bool| {
        axioval_ir::ExplanationEntry {
            path: path.to_owned(),
            kind: kind.to_owned(),
            label: label.map(str::to_owned),
            value: Some(value.to_owned()),
            not_evaluated: None,
            deciding,
        }
    };
    let mut report = report();
    let mut finding = Finding::new(
        RuleId::new("cover").unwrap(),
        wall(3),
        Severity::Error,
        "requirement does not hold",
    );
    finding.explanation = Some(axioval_ir::Explanation {
        entries: vec![
            step("requirement.and[0]", "isDefined", None, "true", false),
            step(
                "requirement.and[1].compare.left",
                "property",
                Some("cover"),
                "0.035 m",
                true,
            ),
            step("requirement.and[1]", "compare", None, "false", true),
            step("requirement", "and", None, "false", true),
        ],
        truncated: false,
    });
    report.findings.push(finding);
    let html = render(&report, &project(), &Template::default(), &options());
    assert!(
        html.contains(
            "<ol class=\"why\"><li>cover (requirement.and[1].compare.left) = 0.035 m</li>\
             <li>requirement.and[1] (compare) = false</li>\
             <li>requirement (and) = false</li></ol>"
        ),
        "{html}"
    );
    assert!(
        !html.contains("isDefined"),
        "only the deciding path is shown"
    );
}
