//! ifcXML is read into the same model, and so the same IR, as its STEP form.
#![allow(missing_docs)]

use std::sync::Arc;

use axioval_engine::{
    CoordinateSystemServiceHandle, EvidenceSession, NameMatch, PropertyEnumerationRequest,
    PropertyResolutionServiceHandle,
};
use axioval_ifc::{IfcSessionError, import_ifc_session, import_ifc_xml_session, is_ifc_xml};
use axioval_ir::{ObjectId, Property};
use ifc_model::Codec;
use ifc_step::StepCodec;
use ifc_xml::XmlCodec;

/// A georeferenced project in millimetres: a site, a storey holding a wall
/// with a property set, a quantity and a type. Names that look like numbers
/// (`1`) and references (`i5`) are text, as STEP states them.
const STEP: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','2026-01-01T00:00:00',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCCARTESIANPOINT((0.,0.,0.));
#4=IFCAXIS2PLACEMENT3D(#3,$,$);
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#4,$);
#6=IFCPROJECT('0000000000000000000006',$,'P',$,$,$,$,(#5),#2);
#7=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#8=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,#7);
#9=IFCMAPCONVERSION(#5,#8,500000.,5600000.,50.,$,$,$);
#10=IFCLOCALPLACEMENT($,#4);
#11=IFCSITE('0000000000000000000011',$,'Site',$,$,#10,$,$,.ELEMENT.,$,$,$,$,$);
#12=IFCBUILDINGSTOREY('0000000000000000000012',$,'1',$,$,#10,$,$,.ELEMENT.,3000.);
#13=IFCWALL('0000000000000000000013',$,'i5',$,$,#10,$,$,.STANDARD.);
#14=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000014',$,$,$,(#13),#12);
#15=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.T.),$);
#16=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI 60'),$);
#17=IFCPROPERTYSINGLEVALUE('Width',$,IFCLENGTHMEASURE(240.),$);
#18=IFCPROPERTYSET('0000000000000000000018',$,'Pset_WallCommon',$,(#15,#16,#17));
#19=IFCRELDEFINESBYPROPERTIES('0000000000000000000019',$,$,$,(#13),#18);
#20=IFCQUANTITYLENGTH('Length',$,$,4000.,$);
#21=IFCELEMENTQUANTITY('0000000000000000000021',$,'Qto_WallBaseQuantities',$,$,(#20));
#22=IFCRELDEFINESBYPROPERTIES('0000000000000000000022',$,$,$,(#13),#21);
#23=IFCWALLTYPE('0000000000000000000023',$,'T',$,$,$,$,$,$,.STANDARD.);
#24=IFCRELDEFINESBYTYPE('0000000000000000000024',$,$,$,(#13),#23);
ENDSEC;
END-ISO-10303-21;
";

/// The model written as ifcXML by `codec`.
fn xml(codec: &XmlCodec) -> Vec<u8> {
    let model = StepCodec.read_bytes(STEP.as_bytes()).unwrap();
    codec.write_bytes(&model).unwrap()
}

fn named() -> XmlCodec {
    XmlCodec::with_schema(Arc::new(ifc_schema::ifc4().clone()))
}

/// Everything a rule can read from the session, with the source left out:
/// each object's kind, identities and properties, and the coordinate system.
fn ir(session: &EvidenceSession) -> Vec<String> {
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    let mut facts = Vec::new();
    for object in session.project().objects() {
        let mut external: Vec<String> = object
            .external_ids
            .iter()
            .map(|id| format!("{id:?}"))
            .collect();
        external.sort();
        facts.push(format!(
            "{} {} {external:?}",
            object.id.local_id,
            object.kind()
        ));
        let request =
            PropertyEnumerationRequest::try_new(object.id.clone(), NameMatch::Any, NameMatch::Any)
                .unwrap();
        let enumeration = properties.enumerate(&request).unwrap();
        for Property {
            property_set,
            name,
            value,
            data_type,
            ..
        } in enumeration.properties()
        {
            facts.push(format!(
                "  {property_set}.{name} = {value:?} ({data_type:?})"
            ));
        }
    }
    let source = session.snapshots().next().unwrap().source().clone();
    let system = session
        .service::<CoordinateSystemServiceHandle>()
        .unwrap()
        .coordinate_system(&source)
        .unwrap();
    facts.push(format!(
        "{:?} {:?} {:?} {:?}",
        system.world(),
        system.true_north(),
        system.map(),
        system.site()
    ));
    facts
}

#[test]
fn an_ifcxml_model_reads_into_the_same_ir_as_its_step_form() {
    let step = import_ifc_session("model.ifc", STEP.as_bytes()).unwrap();
    let expected = ir(&step);
    assert!(
        expected.iter().any(|fact| fact.contains("FireRating")),
        "{expected:#?}"
    );
    for (label, codec) in [
        ("schema names", named()),
        ("positional", XmlCodec::default()),
    ] {
        let bytes = xml(&codec);
        assert!(is_ifc_xml(&bytes));
        let session = import_ifc_xml_session("model.ifcxml", &bytes)
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(ir(&session), expected, "{label}");
        let object = session.project().objects().next().unwrap();
        assert_eq!(
            object.id,
            ObjectId::new(object.id.source.clone(), object.id.local_id.clone()).unwrap()
        );
        assert_eq!(object.id.source.system, "ifc-xml");
        assert_eq!(object.id.source.document, "model.ifcxml");
    }
}

#[test]
fn entity_names_in_any_case_and_omitted_trailing_attributes_read_as_step() {
    let bytes = String::from_utf8(xml(&named())).unwrap();
    // Mixed-case element names, and the site's unset trailing `SiteAddress`
    // left out.
    let edited = bytes
        .replace("<IFCWALL ", "<IfcWall ")
        .replace("</IFCWALL>", "</IfcWall>");
    assert_ne!(edited, bytes);
    let omitted = edited.replace("<SiteAddress xsi:nil=\"true\"/>", "");
    assert_ne!(omitted, edited, "{edited}");
    let edited = omitted;
    let session = import_ifc_xml_session("model.ifcxml", edited.as_bytes()).unwrap();
    let step = import_ifc_session("model.ifc", STEP.as_bytes()).unwrap();
    assert_eq!(ir(&session), ir(&step));
}

fn refusal(xml: &str) -> IfcSessionError {
    match import_ifc_xml_session("model.ifcxml", xml.as_bytes()) {
        Ok(_) => panic!("read: {xml}"),
        Err(error) => error,
    }
}

#[test]
fn a_document_not_read_as_it_states_is_refused() {
    // The buildingSMART XSD arrangement: the placement is a nested entity
    // element with a `ref`, which the codec would read as an empty value.
    let nested = r#"<?xml version="1.0"?>
<ifcXML xmlns="http://www.buildingsmart-tech.org/ifcXML/IFC4/final" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" schema="IFC4">
  <IfcLocalPlacement id="i10"/>
  <IfcWall id="i13" GlobalId="0000000000000000000013">
    <ObjectPlacement><IfcLocalPlacement ref="i10" xsi:nil="true"/></ObjectPlacement>
  </IfcWall>
</ifcXML>"#;
    assert!(
        matches!(refusal(nested), IfcSessionError::Xml(reason) if reason.contains("conform")),
        "{:?}",
        refusal(nested)
    );
    // A name that looks like a number is read as one: not an IfcLabel.
    let inferred = r#"<?xml version="1.0"?>
<ifcXML schema="IFC4"><IFCWALL id="i13" GlobalId="0000000000000000000013" Name="1"/></ifcXML>"#;
    assert!(matches!(refusal(inferred), IfcSessionError::Xml(_)));
    // A value under a name no attribute has: the codec would place it in
    // the next free slot.
    let surplus = r#"<?xml version="1.0"?>
<ifcXML schema="IFC4"><IFCWALL id="i13" GlobalId="0000000000000000000013" Extra="x"/></ifcXML>"#;
    assert!(
        matches!(refusal(surplus), IfcSessionError::Xml(reason) if reason.contains("`Extra`, which names no attribute"))
    );
    let unknown = r#"<?xml version="1.0"?>
<ifcXML schema="IFC4"><IFCNOTANENTITY id="i1"/></ifcXML>"#;
    assert!(
        matches!(refusal(unknown), IfcSessionError::Xml(reason) if reason.contains("does not declare"))
    );
    let unsupported = r#"<?xml version="1.0"?><ifcXML schema="IFC5"/>"#;
    assert!(matches!(
        refusal(unsupported),
        IfcSessionError::UnsupportedSchema(_)
    ));
    assert!(matches!(refusal("<ifcXML"), IfcSessionError::Xml(_)));
    // Bounded before the codec parses: a tag of many attributes or namespace
    // declarations.
    let many = " x=\"1\"".repeat(300);
    let crowded = format!("<?xml version=\"1.0\"?><ifcXML schema=\"IFC4\"{many}/>");
    assert!(
        matches!(refusal(&crowded), IfcSessionError::Xml(reason) if reason.contains("start tag"))
    );
    let declarations: String = (0..9)
        .map(|i| [" xmlns:n", &i.to_string(), "=\"urn:x\""].concat())
        .collect();
    let crowded = format!("<?xml version=\"1.0\"?><ifcXML schema=\"IFC4\"{declarations}/>");
    assert!(
        matches!(refusal(&crowded), IfcSessionError::Xml(reason) if reason.contains("start tag"))
    );
}
