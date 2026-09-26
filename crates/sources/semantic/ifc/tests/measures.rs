//! Measures resolve through their effective unit to canonical SI quantities.
#![allow(missing_docs)]

use axioval_engine::{
    EvidenceSession, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ATTRIBUTE_SET, ObjectId, PropertyValue, QuantityDimension, SourceId};

/// Millimetre lengths, square-metre areas, degree angles; one wall with a
/// property set of measures, and a storey at 3000 mm.
const DATA: &str = "\
#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#2=IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.);
#3=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#4=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#5=IFCDIMENSIONALEXPONENTS(0,0,0,0,0,0,0);
#6=IFCMEASUREWITHUNIT(IFCPLANEANGLEMEASURE(0.017453292519943295),#4);
#7=IFCCONVERSIONBASEDUNIT(#5,.PLANEANGLEUNIT.,'DEGREE',#6);
#8=IFCUNITASSIGNMENT((#1,#2,#7));
#9=IFCPROJECT('0000000000000000000009',$,'P',$,$,$,$,$,#8);
#10=IFCBUILDINGSTOREY('000000000000000000000A',$,'1',$,$,$,$,$,.ELEMENT.,3000.);
#11=IFCWALL('000000000000000000000B',$,'W',$,$,$,$,$,$);
#20=IFCPROPERTYSINGLEVALUE('Width',$,IFCPOSITIVELENGTHMEASURE(240.),$);
#21=IFCPROPERTYSINGLEVALUE('Height',$,IFCLENGTHMEASURE(2.5),#3);
#22=IFCPROPERTYSINGLEVALUE('Area',$,IFCAREAMEASURE(12.5),$);
#23=IFCPROPERTYSINGLEVALUE('Slope',$,IFCPLANEANGLEMEASURE(90.),$);
#24=IFCPROPERTYSINGLEVALUE('Share',$,IFCPOSITIVERATIOMEASURE(0.5),$);
#25=IFCPROPERTYSINGLEVALUE('ThermalTransmittance',$,IFCTHERMALTRANSMITTANCEMEASURE(0.24),$);
#26=IFCPROPERTYSINGLEVALUE('Count',$,IFCINTEGER(3),#3);
#30=IFCPROPERTYSET('000000000000000000000U',$,'Pset_Test',$,(#20,#21,#22,#23,#24,#25,#26));
#31=IFCRELDEFINESBYPROPERTIES('000000000000000000000V',$,$,$,(#11),#30);
";

fn session() -> EvidenceSession {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{DATA}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    import_ifc_session("model.ifc", bytes.as_bytes()).unwrap()
}

fn resolve(
    local: &str,
    set: &str,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    let request = PropertyRequest::try_new(object, Some(set.into()), name).unwrap();
    session()
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn value(local: &str, set: &str, name: &str) -> PropertyValue {
    match resolve(local, set, name).unwrap() {
        PropertyResolution::Present(resolved) => resolved.property().value.clone(),
        PropertyResolution::Absent(_) => panic!("{name} is absent"),
    }
}

fn quantity(value: PropertyValue) -> (f64, QuantityDimension) {
    match value {
        PropertyValue::Quantity { value, dimension } => (value, dimension),
        other => panic!("not a quantity: {other:?}"),
    }
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0)
}

#[test]
fn a_project_default_unit_converts_to_metres() {
    let (width, dimension) = quantity(value("#11", "Pset_Test", "Width"));
    assert_eq!(dimension, QuantityDimension::Length);
    assert!(close(width, 0.24), "{width}");
    // The storey's elevation attribute takes the same project unit.
    let (elevation, dimension) = quantity(value("#10", ATTRIBUTE_SET, "Elevation"));
    assert_eq!(dimension, QuantityDimension::Length);
    assert!(close(elevation, 3.0), "{elevation}");
}

#[test]
fn an_explicit_unit_wins_over_the_project_default() {
    let (height, _) = quantity(value("#11", "Pset_Test", "Height"));
    assert!(close(height, 2.5), "{height}");
}

#[test]
fn areas_and_angles_keep_their_own_dimension() {
    assert_eq!(
        quantity(value("#11", "Pset_Test", "Area")),
        (12.5, QuantityDimension::Area)
    );
    let (slope, dimension) = quantity(value("#11", "Pset_Test", "Slope"));
    assert_eq!(dimension, QuantityDimension::PlaneAngle);
    assert!(close(slope, std::f64::consts::FRAC_PI_2), "{slope}");
}

#[test]
fn a_ratio_is_a_plain_number() {
    assert_eq!(
        value("#11", "Pset_Test", "Share"),
        PropertyValue::Decimal(0.5)
    );
}

#[test]
fn a_measure_without_any_applicable_unit_is_refused() {
    // The project assigns no thermal transmittance unit.
    assert!(matches!(
        resolve("#11", "Pset_Test", "ThermalTransmittance"),
        Err(PropertyResolutionError::Incomplete(message))
            if message.contains("IFCTHERMALTRANSMITTANCEMEASURE")
    ));
}

#[test]
fn a_unit_on_a_plain_scalar_is_still_refused() {
    assert_eq!(
        resolve("#11", "Pset_Test", "Count"),
        Err(PropertyResolutionError::InexactEvidence)
    );
}
