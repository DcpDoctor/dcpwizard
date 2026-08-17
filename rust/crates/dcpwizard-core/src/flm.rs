//! parse an ISDCF FLM-x (Facility List Message, SMPTE ST 430-7) document into
//! cinema/screen/cert data. read-only structural parse; matched on local names
//! so the flm namespace revision ("20XX") does not have to be pinned. cert
//! selection (leaf vs CA) is left to the cinema importer.

/// one screen parsed from an FLM auditorium/device. `cert_pems` holds every
/// X.509 cert found for the device (root/intermediate/leaf), each already
/// wrapped as PEM; the importer picks the recipient leaf.
pub struct FlmScreen {
    pub name: String,
    pub cert_pems: Vec<String>,
}

pub struct FlmCinema {
    pub name: String,
    pub emails: Vec<String>,
    pub screens: Vec<FlmScreen>,
}

fn child_text<'a>(node: roxmltree::Node<'a, 'a>, local: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == local)
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn descendants_named<'a>(
    node: roxmltree::Node<'a, 'a>,
    local: &'a str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'a>> {
    node.descendants()
        .filter(move |c| c.is_element() && c.tag_name().name() == local)
}

/// parse the bytes of an FLM document. errors loudly if the root element is not
/// a FacilityListMessage, so a wrong file type fails clearly instead of
/// importing nothing.
pub fn parse(xml: &str) -> Result<FlmCinema, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("FLM is not valid XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "FacilityListMessage" {
        return Err(format!(
            "not an FLM document: root element is <{}>, expected <FacilityListMessage>",
            root.tag_name().name()
        ));
    }

    let facility = descendants_named(root, "FacilityInfo")
        .next()
        .ok_or("FLM has no FacilityInfo element")?;
    let name =
        child_text(facility, "FacilityName").ok_or("FLM FacilityInfo has no FacilityName")?;

    let mut emails = Vec::new();
    for contact in descendants_named(facility, "Contact") {
        if let Some(e) = child_text(contact, "Email")
            && !emails.contains(&e)
        {
            emails.push(e);
        }
    }

    let mut screens = Vec::new();
    for aud in descendants_named(root, "Auditorium") {
        let aud_name = child_text(aud, "AuditoriumNumberOrName")
            .unwrap_or_else(|| format!("screen-{}", screens.len() + 1));
        let devices: Vec<_> = descendants_named(aud, "Device").collect();
        for dev in devices {
            let cert_pems: Vec<String> = descendants_named(dev, "X509Certificate")
                .filter_map(|c| {
                    // gather every text child: FLM samples put an xml comment
                    // before the base64, which splits the text nodes.
                    let b64: String = c
                        .descendants()
                        .filter(|n| n.is_text())
                        .filter_map(|n| n.text())
                        .collect();
                    if b64.split_whitespace().next().is_none() {
                        None
                    } else {
                        Some(postkit::certificate::der_base64_to_pem(&b64))
                    }
                })
                .collect();
            if cert_pems.is_empty() {
                continue;
            }
            // name a screen by its auditorium, disambiguating by device serial
            // when an auditorium holds more than one device.
            let name = match child_text(dev, "DeviceSerial") {
                Some(s) if descendants_named(aud, "Device").count() > 1 => {
                    format!("{aud_name} ({s})")
                }
                _ => aud_name.clone(),
            };
            screens.push(FlmScreen { name, cert_pems });
        }
    }

    if screens.is_empty() {
        return Err("FLM has no auditoriums with device certificates".to_string());
    }

    Ok(FlmCinema {
        name,
        emails,
        screens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // a minimal FLM matching the SMPTE 430-7 shape (MovieLabs FLM-x sample).
    const FLM: &str = r#"<?xml version="1.0"?>
<flm:FacilityListMessage xmlns:flm="http://www.smpte-ra.org/schemas/430-7/20XX/FLM"
    xmlns:ds="http://www.w3.org/2000/09/xmldsig#">
  <flm:FacilityInfo>
    <flm:FacilityName>Contoso 20</flm:FacilityName>
    <flm:ContactList>
      <flm:Contact><flm:Email>jdoe@contoso.biz</flm:Email></flm:Contact>
      <flm:Contact><flm:Email>ops@contoso.biz</flm:Email></flm:Contact>
    </flm:ContactList>
  </flm:FacilityInfo>
  <flm:AuditoriumList>
    <flm:Auditorium>
      <flm:AuditoriumNumberOrName>1</flm:AuditoriumNumberOrName>
      <flm:SuiteList><flm:Suite><flm:Device>
        <flm:DeviceSerial>218281828</flm:DeviceSerial>
        <flm:KeyInfoList><ds:KeyInfo><ds:X509Data>
          <ds:X509Certificate>QUJDREVG</ds:X509Certificate>
        </ds:X509Data></ds:KeyInfo></flm:KeyInfoList>
      </flm:Device></flm:Suite></flm:SuiteList>
    </flm:Auditorium>
  </flm:AuditoriumList>
</flm:FacilityListMessage>"#;

    #[test]
    fn parses_facility_contacts_and_screen() {
        let c = parse(FLM).unwrap();
        assert_eq!(c.name, "Contoso 20");
        assert_eq!(c.emails, vec!["jdoe@contoso.biz", "ops@contoso.biz"]);
        assert_eq!(c.screens.len(), 1);
        assert_eq!(c.screens[0].name, "1");
        assert!(c.screens[0].cert_pems[0].contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn rejects_non_flm() {
        assert!(parse("<something/>").is_err());
        assert!(parse("not xml <<<").is_err());
    }
}
