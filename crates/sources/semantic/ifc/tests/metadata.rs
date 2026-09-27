//! What a file states about itself: its authoring applications, from
//! `IfcOwnerHistory`, its project's name and its schema.
#![allow(missing_docs)]

use axioval_engine::SourceMetadata;
use axioval_ifc::import_ifc_session;
use axioval_ir::SourceId;
use axioval_ir::contract::SourceField;

const PEOPLE: &str = "\
#40=IFCPERSON($,'Doe','Jane',$,$,$,$,$);
#41=IFCORGANIZATION($,'Firm',$,$,$);
#42=IFCPERSONANDORGANIZATION(#40,#41,$);
";

fn file(schema: &str, data: &str) -> Vec<u8> {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{PEOPLE}{data}#99=IFCWALL('0000000000000000000099',$,$,$,$,$,$,$,$);\nENDSEC;\nEND-ISO-10303-21;\n"
    )
    .into_bytes()
}

fn metadata(schema: &str, data: &str) -> Option<SourceMetadata> {
    let session = import_ifc_session("m.ifc", &file(schema, data)).unwrap();
    let source = SourceId::new("ifc-step", "m.ifc").unwrap();
    session.source_metadata(&source).cloned()
}

fn values(metadata: &SourceMetadata, field: SourceField) -> Option<Vec<&str>> {
    metadata
        .values(field)
        .map(|values| values.iter().map(String::as_str).collect())
}

#[test]
fn owning_applications_and_the_project_name_are_read() {
    let data = "\
#43=IFCAPPLICATION(#41,'2024','Modeller Architecture 2024','MA');
#44=IFCAPPLICATION(#41,'1.0','Converter','CV');
#45=IFCOWNERHISTORY(#42,#43,$,.ADDED.,$,$,$,0);
#46=IFCOWNERHISTORY(#42,#44,$,.ADDED.,$,$,$,0);
#47=IFCOWNERHISTORY(#42,#43,$,.ADDED.,$,$,$,0);
#3=IFCPROJECT('0000000000000000000003',#45,'Clinic',$,$,$,$,$,$);
";
    let metadata = metadata("IFC4", data).unwrap();
    assert_eq!(
        values(&metadata, SourceField::Application),
        Some(vec!["Converter", "Modeller Architecture 2024"])
    );
    assert_eq!(
        values(&metadata, SourceField::Project),
        Some(vec!["Clinic"])
    );
    // The schema is the snapshot's and the file name the host's.
    assert_eq!(metadata.values(SourceField::Schema), None);
    assert_eq!(metadata.values(SourceField::FileName), None);
}

#[test]
fn ifc2x3_owner_histories_are_read_through_their_release() {
    let data = "\
#43=IFCAPPLICATION(#41,'2010','Modeller Structure 2010','MS');
#45=IFCOWNERHISTORY(#42,#43,$,.ADDED.,$,$,$,0);
";
    let metadata = metadata("IFC2X3", data).unwrap();
    assert_eq!(
        values(&metadata, SourceField::Application),
        Some(vec!["Modeller Structure 2010"])
    );
    // No project: the file states none.
    assert_eq!(values(&metadata, SourceField::Project), Some(vec![]));
}

#[test]
fn a_file_without_owner_histories_states_no_application() {
    let metadata = metadata("IFC4", "").unwrap();
    assert_eq!(values(&metadata, SourceField::Application), Some(vec![]));
}

#[test]
fn an_unreadable_application_leaves_the_field_unread() {
    // The owning application is not an `IfcApplication`.
    let data = "#45=IFCOWNERHISTORY(#42,#41,$,.ADDED.,$,$,$,0);\n";
    let metadata = metadata("IFC4", data).unwrap();
    assert_eq!(metadata.values(SourceField::Application), None);
    assert_eq!(values(&metadata, SourceField::Project), Some(vec![]));
}
