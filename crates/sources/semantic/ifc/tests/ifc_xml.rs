//! ifcXML is read into the same model, and so the same IR, as its STEP form.
#![allow(missing_docs)]

use std::sync::Arc;

use axioval_engine::{
    CoordinateSystemServiceHandle, EvidenceSession, NameMatch, PropertyEnumerationRequest,
    PropertyResolutionServiceHandle,
};
use axioval_ifc::{
    IfcSessionError, import_ifc_session, import_ifc_xml_session, is_ifc_xml, read_ifc_xml,
};
use axioval_ir::{ObjectId, Property};
use ifc_model::{Codec, EntityId, Value};
use ifc_step::StepCodec;
use ifc_xml::{XmlCodec, XmlProfile};

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
        (
            "release profile",
            XmlCodec::with_schema_and_profile(
                Arc::new(ifc_schema::ifc4().clone()),
                XmlProfile::Ifc4Add2Tc1,
            ),
        ),
        // The buildingSMART XSD configuration, as `ifc-xml` writes it.
        (
            "XSD configuration",
            XmlCodec::xsd(
                Arc::new(ifc_schema::ifc4().clone()),
                XmlProfile::Ifc4Add2Tc1,
            ),
        ),
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
    // The codec's own layout is read strictly: the XSD arrangement under a
    // `schema` attribute, a nested entity element with a `ref`, is no value
    // of that layout.
    let nested = r#"<?xml version="1.0"?>
<ifcXML xmlns="http://www.buildingsmart-tech.org/ifcXML/IFC4/final" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" schema="IFC4">
  <IfcLocalPlacement id="i10"/>
  <IfcWall id="i13" GlobalId="0000000000000000000013">
    <ObjectPlacement><IfcLocalPlacement ref="i10" xsi:nil="true"/></ObjectPlacement>
  </IfcWall>
</ifcXML>"#;
    assert!(
        matches!(refusal(nested), IfcSessionError::Xml(reason) if reason.contains("ObjectPlacement")),
        "{:?}",
        refusal(nested)
    );
    // Positional names (`a0`, ...) say nothing about which attribute a value
    // is: refused, never placed by position.
    let positional = String::from_utf8(xml(&XmlCodec::default())).unwrap();
    assert!(
        matches!(refusal(&positional), IfcSessionError::Xml(reason) if reason.contains("no attribute `a0`")),
        "{:?}",
        refusal(&positional)
    );
    // A value under a name no attribute has.
    let surplus = r#"<?xml version="1.0"?>
<ifcXML schema="IFC4"><IFCWALL id="i13" GlobalId="0000000000000000000013" Extra="x"/></ifcXML>"#;
    assert!(
        matches!(refusal(surplus), IfcSessionError::Xml(reason) if reason.contains("no attribute `Extra`")),
        "{:?}",
        refusal(surplus)
    );
    // A value its attribute's type does not admit.
    let mistyped = r#"<?xml version="1.0"?>
<ifcXML schema="IFC4"><IFCBUILDINGSTOREY id="i1" GlobalId="0000000000000000000013" Elevation="high"/></ifcXML>"#;
    assert!(
        matches!(refusal(mistyped), IfcSessionError::Xml(_)),
        "{:?}",
        refusal(mistyped)
    );
    let unknown = r#"<?xml version="1.0"?>
<ifcXML schema="IFC4"><IFCNOTANENTITY id="i1"/></ifcXML>"#;
    assert!(
        matches!(refusal(unknown), IfcSessionError::Xml(reason) if reason.contains("IFCNOTANENTITY")),
        "{:?}",
        refusal(unknown)
    );
    let unsupported = r#"<?xml version="1.0"?><ifcXML schema="IFC5"/>"#;
    assert!(matches!(
        refusal(unsupported),
        IfcSessionError::UnsupportedSchema(_)
    ));
    // Neither layout: no `schema`, and no namespace of a release the XSD
    // reader reads (IFC2X3's ifcXML namespace included).
    for root in [
        r#"<?xml version="1.0"?><ifcXML/>"#,
        r#"<?xml version="1.0"?><ifcXML xmlns="http://www.iai-tech.org/ifcXML/IFC2x3/FINAL"/>"#,
    ] {
        assert!(
            matches!(refusal(root), IfcSessionError::Xml(reason) if reason.contains("names no `schema`")),
            "{root}: {:?}",
            refusal(root)
        );
    }
    let other = r#"<?xml version="1.0"?><model schema="IFC4"/>"#;
    assert!(
        matches!(refusal(other), IfcSessionError::Xml(reason) if reason.contains("not `ifcXML`")),
        "{:?}",
        refusal(other)
    );
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

#[test]
fn a_label_that_looks_like_a_number_is_read_as_its_declaration_states() {
    // The strict read types `Name` from its declaration (`IfcLabel`), never
    // from its text, as STEP's `'1'` states it.
    let bytes = r#"<?xml version="1.0"?>
<ifcXML schema="IFC4"><IFCWALL id="i13" GlobalId="0000000000000000000013" Name="1"/></ifcXML>"#;
    let model = read_ifc_xml(bytes.as_bytes()).unwrap();
    assert_eq!(
        model.get(EntityId(13)).unwrap().attributes[2],
        Value::Text("1".into())
    );
}

/// A georeferenced project in the buildingSMART XSD configuration of IFC4
/// ADD2 TC1: entities nested in place, `ref` references with `xsi:nil`,
/// `xsi:type` for subtypes in abstract slots, an inverse attribute
/// (`ContainsElements`) and `-wrapper` values. The codec numbers entities in
/// document order, as [`XSD_STEP`] numbers them.
const XSD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ifc:ifcXML xmlns:ifc="https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/XML" xmlns="https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/XML" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <header>
    <name>n</name>
    <time_stamp>2026-01-01T00:00:00</time_stamp>
  </header>
  <IfcProject id="project" GlobalId="0000000000000000000001" Name="P">
    <RepresentationContexts>
      <IfcGeometricRepresentationContext id="context" ContextType="Model" CoordinateSpaceDimension="3" Precision="1.E-05">
        <WorldCoordinateSystem>
          <IfcAxis2Placement3D id="origin">
            <Location Coordinates="0. 0. 0."/>
          </IfcAxis2Placement3D>
        </WorldCoordinateSystem>
      </IfcGeometricRepresentationContext>
    </RepresentationContexts>
    <UnitsInContext>
      <Units>
        <IfcSIUnit UnitType="lengthunit" Prefix="milli" Name="metre"/>
      </Units>
    </UnitsInContext>
  </IfcProject>
  <IfcMapConversion Eastings="500000." Northings="5600000." OrthogonalHeight="50.">
    <SourceCRS>
      <IfcGeometricRepresentationContext ref="context" xsi:nil="true"/>
    </SourceCRS>
    <TargetCRS xsi:type="IfcProjectedCRS" Name="EPSG:25832">
      <MapUnit xsi:type="IfcSIUnit" UnitType="lengthunit" Name="metre"/>
    </TargetCRS>
  </IfcMapConversion>
  <IfcSite GlobalId="0000000000000000000010" Name="Site" CompositionType="element">
    <ObjectPlacement xsi:type="IfcLocalPlacement" id="placement">
      <RelativePlacement>
        <IfcAxis2Placement3D ref="origin" xsi:nil="true"/>
      </RelativePlacement>
    </ObjectPlacement>
  </IfcSite>
  <IfcBuildingStorey GlobalId="0000000000000000000012" Name="1" CompositionType="element" Elevation="3000.">
    <ObjectPlacement xsi:type="IfcLocalPlacement" ref="placement" xsi:nil="true"/>
    <ContainsElements>
      <IfcRelContainedInSpatialStructure GlobalId="0000000000000000000013">
        <RelatedElements>
          <IfcWall id="wall" GlobalId="0000000000000000000014" Name="i5" PredefinedType="standard">
            <ObjectPlacement xsi:type="IfcLocalPlacement" ref="placement" xsi:nil="true"/>
          </IfcWall>
        </RelatedElements>
      </IfcRelContainedInSpatialStructure>
    </ContainsElements>
  </IfcBuildingStorey>
  <IfcRelDefinesByProperties GlobalId="0000000000000000000015">
    <RelatedObjects>
      <IfcWall ref="wall" xsi:nil="true"/>
    </RelatedObjects>
    <RelatingPropertyDefinition>
      <IfcPropertySet GlobalId="0000000000000000000016" Name="Pset_WallCommon">
        <HasProperties>
          <IfcPropertySingleValue Name="IsExternal">
            <NominalValue><IfcBoolean-wrapper>true</IfcBoolean-wrapper></NominalValue>
          </IfcPropertySingleValue>
          <IfcPropertySingleValue Name="FireRating">
            <NominalValue><IfcLabel-wrapper>EI 60</IfcLabel-wrapper></NominalValue>
          </IfcPropertySingleValue>
          <IfcPropertySingleValue Name="Width">
            <NominalValue><IfcLengthMeasure-wrapper>240.</IfcLengthMeasure-wrapper></NominalValue>
          </IfcPropertySingleValue>
        </HasProperties>
      </IfcPropertySet>
    </RelatingPropertyDefinition>
  </IfcRelDefinesByProperties>
  <IfcRelDefinesByProperties GlobalId="0000000000000000000020">
    <RelatedObjects>
      <IfcWall ref="wall" xsi:nil="true"/>
    </RelatedObjects>
    <RelatingPropertyDefinition>
      <IfcElementQuantity GlobalId="0000000000000000000021" Name="Qto_WallBaseQuantities">
        <Quantities>
          <IfcQuantityLength Name="Length" LengthValue="4000."/>
        </Quantities>
      </IfcElementQuantity>
    </RelatingPropertyDefinition>
  </IfcRelDefinesByProperties>
  <IfcRelDefinesByType GlobalId="0000000000000000000023">
    <RelatedObjects>
      <IfcWall ref="wall" xsi:nil="true"/>
    </RelatedObjects>
    <RelatingType xsi:type="IfcWallType" GlobalId="0000000000000000000024" Name="T" PredefinedType="standard"/>
  </IfcRelDefinesByType>
</ifc:ifcXML>
"#;

/// [`XSD`] as STEP, numbered in the XSD document's order.
const XSD_STEP: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','2026-01-01T00:00:00',(''),(''),'','','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('0000000000000000000001',$,'P',$,$,$,$,(#2),#5);
#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#3,$);
#3=IFCAXIS2PLACEMENT3D(#4,$,$);
#4=IFCCARTESIANPOINT((0.,0.,0.));
#5=IFCUNITASSIGNMENT((#6));
#6=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#7=IFCMAPCONVERSION(#2,#8,500000.,5600000.,50.,$,$,$);
#8=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,#9);
#9=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#10=IFCSITE('0000000000000000000010',$,'Site',$,$,#11,$,$,.ELEMENT.,$,$,$,$,$);
#11=IFCLOCALPLACEMENT($,#3);
#12=IFCBUILDINGSTOREY('0000000000000000000012',$,'1',$,$,#11,$,$,.ELEMENT.,3000.);
#13=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000013',$,$,$,(#14),#12);
#14=IFCWALL('0000000000000000000014',$,'i5',$,$,#11,$,$,.STANDARD.);
#15=IFCRELDEFINESBYPROPERTIES('0000000000000000000015',$,$,$,(#14),#16);
#16=IFCPROPERTYSET('0000000000000000000016',$,'Pset_WallCommon',$,(#17,#18,#19));
#17=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.T.),$);
#18=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI 60'),$);
#19=IFCPROPERTYSINGLEVALUE('Width',$,IFCLENGTHMEASURE(240.),$);
#20=IFCRELDEFINESBYPROPERTIES('0000000000000000000020',$,$,$,(#14),#21);
#21=IFCELEMENTQUANTITY('0000000000000000000021',$,'Qto_WallBaseQuantities',$,$,(#22));
#22=IFCQUANTITYLENGTH('Length',$,$,4000.,$);
#23=IFCRELDEFINESBYTYPE('0000000000000000000023',$,$,$,(#14),#24);
#24=IFCWALLTYPE('0000000000000000000024',$,'T',$,$,$,$,$,$,.STANDARD.);
ENDSEC;
END-ISO-10303-21;
";

#[test]
fn an_xsd_configuration_document_reads_into_the_ir_of_its_step_form() {
    let step = import_ifc_session("model.ifc", XSD_STEP.as_bytes()).unwrap();
    let expected = ir(&step);
    assert!(
        expected.iter().any(|fact| fact.contains("FireRating")),
        "{expected:#?}"
    );
    assert!(is_ifc_xml(XSD.as_bytes()));
    let session = import_ifc_xml_session("model.ifcxml", XSD.as_bytes())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(ir(&session), expected);
    // Entity by entity, the model the STEP codec reads.
    let model = read_ifc_xml(XSD.as_bytes()).unwrap();
    let from_step = StepCodec.read_bytes(XSD_STEP.as_bytes()).unwrap();
    assert_eq!(model.len(), from_step.len());
    for (id, expected) in from_step.iter() {
        let found = model.get(id).unwrap();
        assert_eq!(found.type_name, expected.type_name, "{id}");
        assert_eq!(found.attributes, expected.attributes, "{id}");
    }
}

#[test]
fn an_xsd_configuration_document_its_release_does_not_admit_is_refused() {
    let edited = |from: &str, to: &str| {
        assert!(XSD.contains(from), "{from}");
        XSD.replacen(from, to, 1)
    };
    // An attribute the entity does not declare.
    let unknown = edited("<IfcSite GlobalId=", "<IfcSite Colour=\"red\" GlobalId=");
    assert!(
        matches!(refusal(&unknown), IfcSessionError::Xml(reason) if reason.contains("no attribute `Colour`")),
        "{:?}",
        refusal(&unknown)
    );
    // A decimal comma is no XSD real.
    let comma = edited("Elevation=\"3000.\"", "Elevation=\"3000,5\"");
    assert!(
        matches!(refusal(&comma), IfcSessionError::Xml(reason) if reason.contains("\"3000,5\"")),
        "{:?}",
        refusal(&comma)
    );
    // A reference to nothing.
    let dangling = edited(
        "<IfcAxis2Placement3D ref=\"origin\"",
        "<IfcAxis2Placement3D ref=\"nowhere\"",
    );
    assert!(
        matches!(refusal(&dangling), IfcSessionError::Xml(reason) if reason.contains("nowhere")),
        "{:?}",
        refusal(&dangling)
    );
}

#[test]
fn an_ifc4x3_xsd_configuration_document_is_read_with_the_ifc4x3_tables() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ifcXML xmlns="https://standards.buildingsmart.org/IFC/RELEASE/IFC4/3/ADD2">
  <IfcWall GlobalId="0000000000000000000001" PredefinedType="standard"/>
  <IfcRoad GlobalId="0000000000000000000002"/>
</ifcXML>"#;
    let step = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('','',(''),(''),'','','');
FILE_SCHEMA(('IFC4X3_ADD2'));
ENDSEC;
DATA;
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,.STANDARD.);
#2=IFCROAD('0000000000000000000002',$,$,$,$,$,$,$,$,$);
ENDSEC;
END-ISO-10303-21;
";
    let model = read_ifc_xml(xml.as_bytes()).unwrap();
    let from_step = StepCodec.read_bytes(step.as_bytes()).unwrap();
    assert_eq!(model.len(), from_step.len());
    for (id, expected) in from_step.iter() {
        let found = model.get(id).unwrap();
        assert_eq!(found.type_name, expected.type_name, "{id}");
        assert_eq!(found.attributes, expected.attributes, "{id}");
    }
    let kinds = |session: &EvidenceSession| {
        session
            .project()
            .objects()
            .map(|object| format!("{} {}", object.id.local_id, object.kind()))
            .collect::<Vec<_>>()
    };
    let session = import_ifc_xml_session("road.ifcxml", xml.as_bytes()).unwrap();
    let expected = import_ifc_session("road.ifc", step.as_bytes()).unwrap();
    assert_eq!(kinds(&session), kinds(&expected));
    assert!(
        kinds(&session)
            .iter()
            .any(|kind| kind.to_ascii_uppercase().contains("IFCROAD")),
        "{:?}",
        kinds(&session)
    );
}
