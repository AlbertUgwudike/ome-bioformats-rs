//! Translation of the CZI metadata XML segment. Port of the image-level
//! parts of `ZeissCZIReader.{translateInformation, translateScaling,
//! translateDisplaySettings}`. Instrument, experiment, ROI and plate
//! metadata are not ported yet.

use std::collections::BTreeMap;

use roxmltree::Node;

use crate::meta::{Quantity, pack_color};

#[derive(Debug, Clone, Default)]
pub struct CziChannel {
    pub name: Option<String>,
    /// `#AARRGGBB` from the display settings, already packed as OME RGBA.
    pub color: Option<i32>,
    pub emission: Option<Quantity>,
    pub excitation: Option<Quantity>,
    pub fluor: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CziMetadata {
    /// `Information/Document/Name`.
    pub name: Option<String>,
    pub description: Option<String>,
    /// ISO 8601, normalised to millisecond precision as Bio-Formats does.
    pub acquisition_date: Option<String>,
    pub bits_per_pixel: Option<u32>,
    /// Pixel sizes in micrometres at full resolution.
    pub physical_x: Option<f64>,
    pub physical_y: Option<f64>,
    pub physical_z: Option<f64>,
    pub channels: Vec<CziChannel>,
    /// Flattened `A|B|C` -> text, for original metadata.
    pub original: BTreeMap<String, String>,
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|c| c.has_tag_name(name))
}

/// Port of `getFirstNode`: the first descendant with this name.
fn first<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.descendants().skip(1).find(|c| c.has_tag_name(name))
}

/// Port of `getFirstNodeValue`.
fn first_text(node: Node, name: &str) -> Option<String> {
    first(node, name)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// Port of `getGrandchildren(root, name)`: `root/*/name`.
fn grandchildren<'a, 'i>(node: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children()
        .filter(Node::is_element)
        .flat_map(move |c| c.children().filter(move |g| g.has_tag_name(name)))
}

/// Port of `normalizeColor` plus the `<< 8 | 0xff` packing.
fn parse_color(color: &str) -> Option<i32> {
    let mut c = color.replace('#', "");
    if c.len() > 6 {
        c = c[2..c.len().min(8)].to_owned();
    }
    let rgb = u32::from_str_radix(&c, 16).ok()?;
    Some(pack_color(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
        255,
    ))
}

fn wavelength(text: Option<String>) -> Option<Quantity> {
    let v: f64 = text?.parse().ok()?;
    (v > 0.0).then(|| Quantity::new(v, "nm"))
}

/// Trims an ISO 8601 timestamp to milliseconds and drops the zone suffix,
/// matching how Bio-Formats' `Timestamp` prints CZI dates.
fn normalize_date(date: &str) -> String {
    let date = date.trim().trim_end_matches('Z');
    let date = date.split(['+']).next().unwrap_or(date);
    match date.split_once('.') {
        Some((head, frac)) => {
            let frac: String = frac
                .chars()
                .take_while(char::is_ascii_digit)
                .take(3)
                .collect();
            if frac.is_empty() {
                head.to_owned()
            } else {
                format!("{head}.{frac}")
            }
        }
        None => date.to_owned(),
    }
}

pub fn parse(xml: &str) -> CziMetadata {
    let mut meta = CziMetadata::default();
    let Ok(doc) = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    ) else {
        return meta;
    };
    let root = doc.root_element();

    if let Some(information) = first(root, "Information") {
        if let Some(image) = child(information, "Image").or_else(|| first(information, "Image")) {
            meta.bits_per_pixel =
                first_text(image, "ComponentBitCount").and_then(|b| b.parse().ok());
            meta.acquisition_date =
                first_text(image, "AcquisitionDateAndTime").map(|d| normalize_date(&d));
            let channels: Vec<Node> = match first(image, "Dimensions") {
                Some(dims) => grandchildren(dims, "Channel").collect(),
                None => image
                    .descendants()
                    .filter(|n| n.has_tag_name("Channel"))
                    .collect(),
            };
            for c in channels {
                meta.channels.push(CziChannel {
                    name: c.attribute("Name").map(str::to_owned),
                    emission: wavelength(first_text(c, "EmissionWavelength")),
                    excitation: wavelength(first_text(c, "ExcitationWavelength")),
                    fluor: first_text(c, "Fluor"),
                    color: None,
                });
            }
        }
        if let Some(document) =
            child(information, "Document").or_else(|| first(information, "Document"))
        {
            meta.description = first_text(document, "Description");
            meta.name = first_text(document, "Name");
        }
    }

    // translateScaling: Scaling/Items/Distance[@Id]/Value, in metres.
    if let Some(scaling) = first(root, "Scaling") {
        let distances = child(scaling, "Items")
            .into_iter()
            .flat_map(|items| items.children().filter(|d| d.has_tag_name("Distance")));
        for distance in distances {
            let Some(value) = first_text(distance, "Value").and_then(|v| v.parse::<f64>().ok())
            else {
                continue;
            };
            let um = value * 1e6;
            if um <= 0.0 {
                continue;
            }
            match distance.attribute("Id") {
                Some("X") => meta.physical_x = Some(um),
                Some("Y") => meta.physical_y = Some(um),
                Some("Z") => meta.physical_z = Some(um),
                _ => {}
            }
        }
    }

    // translateDisplaySettings overrides what Information gave.
    for display in root
        .descendants()
        .filter(|n| n.has_tag_name("DisplaySetting"))
    {
        for (i, c) in grandchildren(display, "Channel").enumerate() {
            if meta.channels.len() <= i {
                meta.channels.resize_with(i + 1, CziChannel::default);
            }
            let ch = &mut meta.channels[i];
            let color = first_text(c, "Color").or_else(|| first_text(c, "OriginalColor"));
            ch.color = color.as_deref().and_then(parse_color);
            if let Some(fluor) = first_text(c, "DyeName") {
                ch.fluor = Some(fluor);
            }
            if let Some(name) = c.attribute("Name") {
                ch.name = Some(name.to_owned());
            }
            if let Some(em) = wavelength(first_text(c, "DyeMaxEmission")) {
                ch.emission = Some(em);
            }
            if let Some(ex) = wavelength(first_text(c, "DyeMaxExcitation")) {
                ch.excitation = Some(ex);
            }
        }
    }

    flatten(root, &mut Vec::new(), &mut meta.original);
    meta
}

/// Flattens leaf text into `Parent|Child` keys (repeats get ` #n`), as a
/// compact stand-in for `populateOriginalMetadata`.
fn flatten(node: Node, stack: &mut Vec<String>, out: &mut BTreeMap<String, String>) {
    let elements: Vec<Node> = node.children().filter(Node::is_element).collect();
    if elements.is_empty() {
        let text = node.text().map(str::trim).unwrap_or("");
        if !text.is_empty() && !stack.is_empty() {
            let key = stack.join("|");
            let key = if out.contains_key(&key) {
                (2..)
                    .map(|n| format!("{key} #{n}"))
                    .find(|k| !out.contains_key(k))
                    .unwrap()
            } else {
                key
            };
            out.insert(key, text.to_owned());
        }
        return;
    }
    for child in elements {
        // Skip the top-level container names to keep keys short.
        let name = child.tag_name().name();
        let skip = matches!(name, "ImageDocument" | "Metadata");
        if !skip {
            stack.push(name.to_owned());
        }
        flatten(child, stack, out);
        if !skip {
            stack.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_colors() {
        assert_eq!(
            normalize_date("2024-05-07T09:03:33.1687902Z"),
            "2024-05-07T09:03:33.168"
        );
        assert_eq!(normalize_date("2024-05-07T09:03:33"), "2024-05-07T09:03:33");
        assert_eq!(parse_color("#FF00FF00"), Some(pack_color(0, 255, 0, 255)));
        assert_eq!(parse_color("#FF0000"), Some(pack_color(255, 0, 0, 255)));
    }

    #[test]
    fn parses_scaling_and_channels() {
        let xml = r##"<ImageDocument><Metadata>
            <Information><Image><ComponentBitCount>12</ComponentBitCount>
              <AcquisitionDateAndTime>2024-05-07T09:03:33.1687902Z</AcquisitionDateAndTime>
              <Dimensions><Channels><Channel Name="DAPI"><EmissionWavelength>461</EmissionWavelength></Channel></Channels></Dimensions>
            </Image></Information>
            <Scaling><Items><Distance Id="X"><Value>6.5E-07</Value></Distance><Distance Id="Z"><Value>2E-06</Value></Distance></Items></Scaling>
            <DisplaySetting><Channels><Channel Name="DAPI"><Color>#FF0000FF</Color></Channel></Channels></DisplaySetting>
            </Metadata></ImageDocument>"##;
        let m = parse(xml);
        assert_eq!(m.bits_per_pixel, Some(12));
        assert!((m.physical_x.unwrap() - 0.65).abs() < 1e-9);
        assert!((m.physical_z.unwrap() - 2.0).abs() < 1e-9);
        assert_eq!(m.channels.len(), 1);
        assert_eq!(m.channels[0].name.as_deref(), Some("DAPI"));
        assert_eq!(m.channels[0].color, Some(pack_color(0, 0, 255, 255)));
        assert_eq!(
            m.channels[0].emission.as_ref().map(|q| q.value),
            Some(461.0)
        );
        assert_eq!(
            m.acquisition_date.as_deref(),
            Some("2024-05-07T09:03:33.168")
        );
    }
}
