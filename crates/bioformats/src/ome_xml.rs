//! OME-XML (2016-06 schema) generation and parsing.
//!
//! Replaces the generated `ome.xml.meta.OMEXMLMetadataImpl` and the
//! `OMEXMLService` helpers for the subset of the model in [`crate::meta`].

use std::fmt::Write as _;

use crate::core::CoreMetadata;
use crate::dims::Zct;
use crate::error::{Error, Result, format_err};
use crate::meta::{ChannelMetadata, ImageMetadata, OmeMetadata, PlaneMetadata, Quantity};
use crate::pixel::PixelType;

pub const OME_NS: &str = "http://www.openmicroscopy.org/Schemas/OME/2016-06";
const ORIGINAL_METADATA_NS: &str = "openmicroscopy.org/OriginalMetadata";
pub const CREATOR: &str = concat!("ome-bioformats-rs ", env!("CARGO_PKG_VERSION"));

/// Where one plane lives in an OME-TIFF.
#[derive(Debug, Clone, PartialEq)]
pub struct TiffData {
    pub ifd: u32,
    pub zct: Zct,
    pub plane_count: u32,
    /// `(FileName, urn:uuid:...)` when the plane lives in a named file.
    pub uuid: Option<(String, String)>,
}

/// Inputs for [`write`] beyond the metadata itself.
pub struct XmlOptions<'a> {
    /// `urn:uuid:...` identifying the document's own file.
    pub uuid: Option<&'a str>,
    /// Per-series TiffData blocks; empty to omit.
    pub tiff_data: &'a [Vec<TiffData>],
    pub original_metadata: bool,
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML 1.0 forbids; Leica metadata sometimes contains them.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn attr(out: &mut String, name: &str, value: impl std::fmt::Display) {
    let _ = write!(out, " {name}=\"{}\"", escape(&value.to_string()));
}

fn opt_attr(out: &mut String, name: &str, value: Option<&str>) {
    if let Some(v) = value {
        attr(out, name, v);
    }
}

fn quantity_attr(out: &mut String, name: &str, q: Option<&Quantity>) {
    if let Some(q) = q
        && q.value.is_finite()
    {
        attr(out, name, q.value);
        attr(out, &format!("{name}Unit"), &q.unit);
    }
}

/// Serializes `meta` (shaped to `cores`) as an OME-XML document.
pub fn write(cores: &[CoreMetadata], meta: &OmeMetadata, opts: &XmlOptions) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    out.push_str("<OME");
    attr(&mut out, "xmlns", OME_NS);
    attr(
        &mut out,
        "xmlns:xsi",
        "http://www.w3.org/2001/XMLSchema-instance",
    );
    attr(
        &mut out,
        "xsi:schemaLocation",
        format!("{OME_NS} {OME_NS}/ome.xsd"),
    );
    attr(&mut out, "Creator", CREATOR);
    opt_attr(&mut out, "UUID", opts.uuid);
    out.push('>');

    let default_image = ImageMetadata::default();
    let mut annotation_refs = Vec::new();
    for (s, core) in cores.iter().enumerate() {
        let image = meta.images.get(s).unwrap_or(&default_image);
        out.push_str("<Image");
        attr(&mut out, "ID", format!("Image:{s}"));
        opt_attr(&mut out, "Name", image.name.as_deref());
        out.push('>');
        if let Some(date) = &image.acquisition_date {
            let _ = write!(out, "<AcquisitionDate>{}</AcquisitionDate>", escape(date));
        }
        if let Some(desc) = &image.description {
            let _ = write!(out, "<Description>{}</Description>", escape(desc));
        }

        out.push_str("<Pixels");
        attr(&mut out, "ID", format!("Pixels:{s}"));
        attr(&mut out, "DimensionOrder", core.dimension_order);
        attr(&mut out, "Type", core.pixel_type.ome_name());
        attr(&mut out, "SignificantBits", core.bits_per_pixel);
        attr(&mut out, "Interleaved", core.interleaved);
        attr(&mut out, "BigEndian", !core.little_endian);
        attr(&mut out, "SizeX", core.size_x);
        attr(&mut out, "SizeY", core.size_y);
        attr(&mut out, "SizeZ", core.size_z);
        attr(&mut out, "SizeC", core.size_c);
        attr(&mut out, "SizeT", core.size_t);
        quantity_attr(&mut out, "PhysicalSizeX", image.physical_size_x.as_ref());
        quantity_attr(&mut out, "PhysicalSizeY", image.physical_size_y.as_ref());
        quantity_attr(&mut out, "PhysicalSizeZ", image.physical_size_z.as_ref());
        quantity_attr(&mut out, "TimeIncrement", image.time_increment.as_ref());
        out.push('>');

        for c in 0..core.effective_size_c() as usize {
            let channel = image.channels.get(c).cloned().unwrap_or_default();
            out.push_str("<Channel");
            attr(&mut out, "ID", format!("Channel:{s}:{c}"));
            opt_attr(&mut out, "Name", channel.name.as_deref());
            attr(&mut out, "SamplesPerPixel", core.rgb_channel_count);
            if let Some(color) = channel.color {
                attr(&mut out, "Color", color);
            }
            opt_attr(&mut out, "Fluor", channel.fluor.as_deref());
            quantity_attr(
                &mut out,
                "ExcitationWavelength",
                channel.excitation_wavelength.as_ref(),
            );
            quantity_attr(
                &mut out,
                "EmissionWavelength",
                channel.emission_wavelength.as_ref(),
            );
            out.push_str("><LightPath/></Channel>");
        }

        let tiff_data = opts.tiff_data.get(s).map(Vec::as_slice).unwrap_or(&[]);
        if tiff_data.is_empty() {
            out.push_str("<MetadataOnly/>");
        }
        for td in tiff_data {
            out.push_str("<TiffData");
            attr(&mut out, "IFD", td.ifd);
            attr(&mut out, "FirstZ", td.zct.z);
            attr(&mut out, "FirstC", td.zct.c);
            attr(&mut out, "FirstT", td.zct.t);
            attr(&mut out, "PlaneCount", td.plane_count);
            match &td.uuid {
                Some((file, uuid)) => {
                    out.push_str("><UUID");
                    attr(&mut out, "FileName", file);
                    let _ = write!(out, ">{}</UUID></TiffData>", escape(uuid));
                }
                None => out.push_str("/>"),
            }
        }

        for plane in &image.planes {
            out.push_str("<Plane");
            attr(&mut out, "TheZ", plane.zct.z);
            attr(&mut out, "TheT", plane.zct.t);
            attr(&mut out, "TheC", plane.zct.c);
            quantity_attr(&mut out, "DeltaT", plane.delta_t.as_ref());
            quantity_attr(&mut out, "ExposureTime", plane.exposure_time.as_ref());
            quantity_attr(&mut out, "PositionX", plane.position_x.as_ref());
            quantity_attr(&mut out, "PositionY", plane.position_y.as_ref());
            quantity_attr(&mut out, "PositionZ", plane.position_z.as_ref());
            out.push_str("/>");
        }
        out.push_str("</Pixels>");
        if opts.original_metadata && !core.series_metadata.is_empty() {
            let _ = write!(out, "<AnnotationRef ID=\"Annotation:Series{s}\"/>");
            annotation_refs.push((format!("Annotation:Series{s}"), &core.series_metadata));
        }
        out.push_str("</Image>");
    }

    if opts.original_metadata && (!meta.global_metadata.is_empty() || !annotation_refs.is_empty()) {
        out.push_str("<StructuredAnnotations>");
        let global = (!meta.global_metadata.is_empty())
            .then(|| ("Annotation:Global".to_string(), &meta.global_metadata));
        for (id, map) in global.into_iter().chain(annotation_refs) {
            out.push_str("<MapAnnotation");
            attr(&mut out, "ID", id);
            attr(&mut out, "Namespace", ORIGINAL_METADATA_NS);
            out.push_str("><Value>");
            for (k, v) in map {
                let _ = write!(out, "<M K=\"{}\">{}</M>", escape(k), escape(v));
            }
            out.push_str("</Value></MapAnnotation>");
        }
        out.push_str("</StructuredAnnotations>");
    }
    out.push_str("</OME>");
    out
}

/// The result of parsing an OME-XML document.
#[derive(Debug, Clone)]
pub struct ParsedOme {
    pub cores: Vec<CoreMetadata>,
    pub meta: OmeMetadata,
    pub tiff_data: Vec<Vec<TiffData>>,
    pub uuid: Option<String>,
}

fn quantity(node: roxmltree::Node, name: &str, default_unit: &str) -> Option<Quantity> {
    let value: f64 = node.attribute(name)?.parse().ok()?;
    let unit = node
        .attribute(format!("{name}Unit").as_str())
        .unwrap_or(default_unit);
    Some(Quantity::new(value, unit))
}

fn parse_u32(node: roxmltree::Node, name: &str, default: u32) -> Result<u32> {
    match node.attribute(name) {
        None => Ok(default),
        Some(v) => v
            .trim()
            .parse()
            .map_err(|_| format_err!("bad {name}={v:?} in OME-XML")),
    }
}

/// Parses the Image/Pixels part of an OME-XML document into core metadata.
pub fn parse(xml: &str) -> Result<ParsedOme> {
    let doc = roxmltree::Document::parse(xml)?;
    let root = doc.root_element();
    if root.tag_name().name() != "OME" {
        return Err(format_err!(
            "root element is <{}>, not <OME>",
            root.tag_name().name()
        ));
    }
    let mut parsed = ParsedOme {
        cores: Vec::new(),
        meta: OmeMetadata::default(),
        tiff_data: Vec::new(),
        uuid: root.attribute("UUID").map(str::to_owned),
    };

    for image in root.children().filter(|n| n.has_tag_name("Image")) {
        let pixels = image
            .children()
            .find(|n| n.has_tag_name("Pixels"))
            .ok_or_else(|| format_err!("Image without Pixels"))?;
        let pixel_type = pixels
            .attribute("Type")
            .and_then(PixelType::from_ome_name)
            .ok_or_else(|| format_err!("missing or invalid Pixels/@Type"))?;
        let mut core = CoreMetadata::new(
            parse_u32(pixels, "SizeX", 0)?,
            parse_u32(pixels, "SizeY", 0)?,
            pixel_type,
        );
        core.size_z = parse_u32(pixels, "SizeZ", 1)?.max(1);
        core.size_c = parse_u32(pixels, "SizeC", 1)?.max(1);
        core.size_t = parse_u32(pixels, "SizeT", 1)?.max(1);
        core.dimension_order = pixels
            .attribute("DimensionOrder")
            .unwrap_or("XYCZT")
            .parse()?;
        core.bits_per_pixel = parse_u32(pixels, "SignificantBits", core.bits_per_pixel)?;
        core.little_endian = pixels.attribute("BigEndian") != Some("true");
        core.interleaved = pixels.attribute("Interleaved") == Some("true");

        let mut image_meta = ImageMetadata {
            name: image.attribute("Name").map(str::to_owned),
            physical_size_x: quantity(pixels, "PhysicalSizeX", "µm"),
            physical_size_y: quantity(pixels, "PhysicalSizeY", "µm"),
            physical_size_z: quantity(pixels, "PhysicalSizeZ", "µm"),
            time_increment: quantity(pixels, "TimeIncrement", "s"),
            ..Default::default()
        };
        for child in image.children().filter(roxmltree::Node::is_element) {
            match child.tag_name().name() {
                "AcquisitionDate" => image_meta.acquisition_date = child.text().map(str::to_owned),
                "Description" => image_meta.description = child.text().map(str::to_owned),
                _ => {}
            }
        }

        let mut samples = 1;
        let mut tiff_data = Vec::new();
        for child in pixels.children().filter(roxmltree::Node::is_element) {
            match child.tag_name().name() {
                "Channel" => {
                    let spp = parse_u32(child, "SamplesPerPixel", 1)?.max(1);
                    samples = spp;
                    image_meta.channels.push(ChannelMetadata {
                        name: child.attribute("Name").map(str::to_owned),
                        samples_per_pixel: spp,
                        color: child.attribute("Color").and_then(|c| c.parse().ok()),
                        excitation_wavelength: quantity(child, "ExcitationWavelength", "nm"),
                        emission_wavelength: quantity(child, "EmissionWavelength", "nm"),
                        fluor: child.attribute("Fluor").map(str::to_owned),
                    });
                }
                "TiffData" => {
                    let uuid = child.children().find(|n| n.has_tag_name("UUID")).map(|u| {
                        (
                            u.attribute("FileName").unwrap_or("").to_owned(),
                            u.text().unwrap_or("").trim().to_owned(),
                        )
                    });
                    tiff_data.push(TiffData {
                        ifd: parse_u32(child, "IFD", 0)?,
                        zct: Zct {
                            z: parse_u32(child, "FirstZ", 0)?,
                            c: parse_u32(child, "FirstC", 0)?,
                            t: parse_u32(child, "FirstT", 0)?,
                        },
                        // 0 means "not given"; resolved by the TIFF reader.
                        plane_count: parse_u32(child, "PlaneCount", 0)?,
                        uuid,
                    });
                }
                "Plane" => image_meta.planes.push(PlaneMetadata {
                    zct: Zct {
                        z: parse_u32(child, "TheZ", 0)?,
                        c: parse_u32(child, "TheC", 0)?,
                        t: parse_u32(child, "TheT", 0)?,
                    },
                    delta_t: quantity(child, "DeltaT", "s"),
                    exposure_time: quantity(child, "ExposureTime", "s"),
                    position_x: quantity(child, "PositionX", "reference frame"),
                    position_y: quantity(child, "PositionY", "reference frame"),
                    position_z: quantity(child, "PositionZ", "reference frame"),
                }),
                _ => {}
            }
        }
        if !core.size_c.is_multiple_of(samples) {
            samples = 1;
        }
        core.rgb_channel_count = samples;
        core.update_image_count();
        parsed.cores.push(core);
        parsed.meta.images.push(image_meta);
        parsed.tiff_data.push(tiff_data);
    }
    if parsed.cores.is_empty() {
        return Err(Error::Format("OME-XML contains no images".into()));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_parse() {
        let mut core = CoreMetadata::new(64, 32, PixelType::Uint16);
        core.size_c = 3;
        core.size_z = 2;
        core.update_image_count();
        core.series_metadata
            .insert("Key <1>".into(), "a & b".into());
        let mut meta = OmeMetadata::default();
        meta.conform_to(std::slice::from_ref(&core));
        meta.images[0].name = Some("my \"image\"".into());
        meta.images[0].physical_size_x = Some(Quantity::micrometers(0.5));
        let td: Vec<TiffData> = (0..core.image_count)
            .map(|i| TiffData {
                ifd: i,
                zct: core.zct(i).unwrap(),
                plane_count: 1,
                uuid: None,
            })
            .collect();
        let xml = write(
            std::slice::from_ref(&core),
            &meta,
            &XmlOptions {
                uuid: Some("urn:uuid:x"),
                tiff_data: std::slice::from_ref(&td),
                original_metadata: true,
            },
        );
        let parsed = parse(&xml).unwrap();
        let mut expected = core.clone();
        expected.series_metadata.clear();
        assert_eq!(parsed.cores, vec![expected]);
        assert_eq!(parsed.tiff_data, vec![td]);
        assert_eq!(parsed.meta.images[0].name.as_deref(), Some("my \"image\""));
        assert_eq!(
            parsed.meta.images[0].physical_size_x,
            Some(Quantity::micrometers(0.5))
        );
        assert_eq!(parsed.uuid.as_deref(), Some("urn:uuid:x"));
    }
}
