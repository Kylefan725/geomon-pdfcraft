#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod fingerprint;

use pdfcraft_cos::{Document, Object, SaveOptions};
use pdfcraft_model::Page;
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, io::BufWriter, path::Path, sync::Arc};

pub const PROCESSING_VERSION: &str = "geomon-pdf/1:2ef2e2b82286acbf4c1ff72fcb7a82ef52fe7a54";
pub const MAX_SOURCE_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("Could not read or store the PDF drawing: {0}")]
    Io(#[from] std::io::Error),
    #[error("The PDF is malformed or unsupported: {0}. Export an unencrypted PDF and upload it again.")]
    Pdf(#[from] pdfcraft_cos::CosError),
    #[error("The PDF cannot be opened: {0}. Export an unencrypted PDF and upload it again.")]
    Open(#[from] pdfcraft_render::OpenError),
    #[error("The selected pages could not be extracted: {0}")]
    Organize(#[from] pdfcraft_organize::OrganizeError),
    #[error("The page image could not be stored: {0}")]
    Png(#[from] png::EncodingError),
    #[error("The drawing metadata could not be read or stored: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize)]
pub struct PageInfo {
    pub index: usize,
    pub identity: String,
    pub media_box: [f64; 4],
    pub crop_box: [f64; 4],
    pub rotation: i64,
    pub user_unit: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Manifest {
    pub source_sha256: String,
    pub processing_version: String,
    pub page_count: usize,
    pub pages: Vec<PageInfo>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RenderManifest {
    #[serde(flatten)]
    pub document: Manifest,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub raster_to_page: [f64; 6],
    pub page_index: usize,
    pub region: [f64; 4],
}

#[derive(Debug, Serialize)]
pub struct Verification {
    pub verified: bool,
    pub reason: String,
}

struct Loaded {
    bytes: Arc<Vec<u8>>,
    document: Document,
    pages: Vec<Page>,
    manifest: Manifest,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn read(path: &Path) -> Result<Vec<u8>> {
    if fs::metadata(path)?.len() > MAX_SOURCE_BYTES {
        return Err(invalid("The PDF exceeds the 100 MiB source limit. Split the drawing and upload it again."));
    }
    let bytes = fs::read(path)?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(invalid("The file is not a PDF. Export a PDF drawing and upload it again."));
    }
    Ok(bytes)
}

fn box_value(document: &Document, value: Option<&Object>) -> Result<[f64; 4]> {
    let resolved = value.map(|v| document.resolve(v)).ok_or_else(|| invalid("The PDF page has no page box. Export it again."))?;
    let array = resolved.as_array().ok_or_else(|| invalid("The PDF page box is not readable. Export it again."))?;
    if array.len() != 4 {
        return Err(invalid("The PDF page box must contain four coordinates."));
    }
    let values: Vec<f64> = array
        .iter()
        .map(|v| document.resolve(v).as_f64().ok_or_else(|| invalid("The PDF page box must contain numeric coordinates.")))
        .collect::<Result<_>>()?;
    let [x0, y0, x1, y1] = values.as_slice() else {
        return Err(invalid("The PDF page box must contain four coordinates."));
    };
    if !values.iter().all(|n| n.is_finite()) || x1 <= x0 || y1 <= y0 {
        return Err(invalid("The PDF page box is empty or invalid. Export it again."));
    }
    Ok([*x0, *y0, *x1, *y1])
}

fn load(bytes: Vec<u8>, max_pages: usize) -> Result<Loaded> {
    if max_pages == 0 || max_pages > 10_000 {
        return Err(invalid("The page limit must be between 1 and 10,000."));
    }
    let bytes = Arc::new(bytes);
    let document = Document::open_with_stream_limit(bytes.clone(), None, 128 * 1024 * 1024)?;
    if document.trailer().contains(b"Encrypt") {
        return Err(invalid("Encrypted PDFs are not supported. Export an unencrypted copy and upload it again."));
    }
    if !document.repair_log().is_empty() {
        return Err(invalid("The PDF needs structural repair. Export a new PDF and upload it again."));
    }
    let pages = pdfcraft_model::pages(&document);
    if pages.is_empty() || pages.len() > max_pages {
        return Err(invalid(format!("The PDF must contain between 1 and {max_pages} pages. Split it and upload it again.")));
    }
    pdfcraft_render::check_pages(bytes.clone(), None, pages.len())?;
    let inspected = pdfcraft_render::inspect(bytes.clone(), None)?;
    if inspected.encrypted || inspected.has_javascript || inspected.xfa.is_some() {
        return Err(invalid(
            "Encrypted PDFs, PDF JavaScript and XFA forms are not supported. Export a static, unencrypted drawing and upload it again.",
        ));
    }
    let mut descriptions = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let media_box = box_value(&document, page.dict.get(b"MediaBox"))?;
        if page.dict.contains(b"CropBox") {
            box_value(&document, page.dict.get(b"CropBox"))?;
        }
        if let Some(value) = page.dict.get(b"Rotate") {
            let value = document.resolve(value);
            if value.as_int().is_none_or(|rotation| rotation % 90 != 0) {
                return Err(invalid("The PDF contains an unsupported page rotation. Export with 0, 90, 180 or 270 degree page rotation."));
            }
        }
        if let Some(value) = page.dict.get(b"UserUnit")
            && document.resolve(value).as_f64().is_none_or(|u| !u.is_finite() || !(1.0..=75_000.0).contains(&u))
        {
            return Err(invalid("The PDF contains unsupported page units. Export a PDF with valid UserUnit values."));
        }
        let (width, height) = page.display_size(&document);
        if !width.is_finite() || !height.is_finite() || width < 0.5 || height < 0.5 {
            return Err(invalid("The PDF contains an empty or unsupported page size. Export it again."));
        }
        let renderer_page = inspected.pages.get(index).ok_or_else(|| invalid("PDF readers disagree on the page tree. Export the PDF again."))?;
        if (f64::from(renderer_page.width) - width).abs() > width * 1e-5 || (f64::from(renderer_page.height) - height).abs() > height * 1e-5 {
            return Err(invalid("PDF readers disagree on the page dimensions. Export the PDF again."));
        }
        descriptions.push(PageInfo {
            index,
            identity: page.obj.to_string(),
            media_box,
            crop_box: page.crop(&document),
            rotation: page.rotation(&document),
            user_unit: page.user_unit(&document),
            width,
            height,
        });
    }
    let manifest = Manifest {
        source_sha256: Sha256::digest(bytes.as_slice()).iter().map(|byte| format!("{byte:02x}")).collect(),
        processing_version: PROCESSING_VERSION.into(),
        page_count: pages.len(),
        pages: descriptions,
        diagnostics: inspected.warnings,
    };
    Ok(Loaded { bytes, document, pages, manifest })
}

pub fn inspect(path: &Path, max_pages: usize) -> Result<Manifest> {
    Ok(load(read(path)?, max_pages)?.manifest)
}

pub fn validate_region(region: [f64; 4], crop: [f64; 4]) -> Result<[f64; 4]> {
    let [x0, y0, x1, y1] = region;
    if !region.iter().all(|n| n.is_finite()) || x1 <= x0 || y1 <= y0 || x0 < crop[0] || y0 < crop[1] || x1 > crop[2] || y1 > crop[3] {
        return Err(invalid("Select a non-empty plan region inside the selected PDF page."));
    }
    Ok(region)
}

pub fn raster_to_page(rotation: i64, region: [f64; 4], unit: f64, scale: f64) -> [f64; 6] {
    let m = pdfcraft_model::view_matrix_for(rotation, region);
    let display_height = if rotation % 180 == 0 { region[3] - region[1] } else { region[2] - region[0] };
    let step = 1.0 / (unit * scale);
    [m[0] * step, m[1] * step, -m[2] * step, -m[3] * step, m[4] + m[2] * display_height, m[5] + m[3] * display_height]
}

pub fn render(
    path: &Path,
    page_index: usize,
    region: Option<[f64; 4]>,
    dpi: f64,
    max_pixels: u64,
    output: &Path,
    max_pages: usize,
) -> Result<RenderManifest> {
    if !dpi.is_finite() || dpi <= 0.0 || dpi > 2400.0 || max_pixels == 0 || max_pixels > 100_000_000 {
        return Err(invalid("Use a positive rendering resolution up to 2400 DPI and a pixel limit up to 100 million."));
    }
    let mut source = load(read(path)?, max_pages)?;
    let page = source.pages.get(page_index).ok_or_else(|| invalid("The selected PDF page does not exist."))?.clone();
    let crop = page.crop(&source.document);
    let region = validate_region(region.unwrap_or(crop), crop)?;
    let rotation = page.rotation(&source.document);
    let unit = page.user_unit(&source.document);
    let (native_width, native_height) = (region[2] - region[0], region[3] - region[1]);
    let (width, height) = if rotation % 180 == 0 { (native_width * unit, native_height * unit) } else { (native_height * unit, native_width * unit) };
    if width < 0.5 || height < 0.5 {
        return Err(invalid("The selected plan region is too small to render."));
    }
    let budget = max_pixels.min(pdfcraft_render::MAX_PIXELS as u64);
    let by_pixels = ((budget as f64) / (width * height)).sqrt();
    let mut scale = pdfcraft_render::effective_scale(width as f32, height as f32, (dpi / 72.0).min(by_pixels) as f32);
    while u64::from(pdfcraft_render::device_pixels(width as f32, scale)) * u64::from(pdfcraft_render::device_pixels(height as f32, scale)) > budget {
        scale *= 0.999;
    }
    if region != crop {
        let mut cropped = Document::open(source.bytes.clone())?;
        cropped.update_dict(page.obj, |dict| dict.set(b"CropBox".to_vec(), Object::Array(region.into_iter().map(Object::Real).collect())))?;
        source.bytes = Arc::new(pdfcraft_cos::write_incremental(&cropped, &SaveOptions::default())?);
    }
    let mut renderer = PageRenderer::new(source.bytes, RenderConfig { transparent: true, reject_oversize: true, ..RenderConfig::default() });
    let rendered = renderer.render(RenderRequest { page: page_index, scale, ..RenderRequest::default() });
    if let Some(error) = rendered.error {
        return Err(invalid(format!("The PDF page could not be rendered: {error}. Export the drawing again.")));
    }
    if u64::from(rendered.width) * u64::from(rendered.height) > budget {
        return Err(invalid("The page image exceeds the pixel budget."));
    }
    let mut pixels = rendered.rgba.to_vec();
    // PDFCraft supplies premultiplied RGBA; PNG stores straight-alpha colour.
    for pixel in pixels.chunks_exact_mut(4) {
        if let [r, g, b, alpha] = pixel
            && *alpha > 0
            && *alpha < 255
        {
            for channel in [r, g, b] {
                *channel = ((u32::from(*channel) * 255 + u32::from(*alpha) / 2) / u32::from(*alpha)).min(255) as u8;
            }
        }
    }
    fs::create_dir_all(output)?;
    let file = fs::File::create(output.join("reference.png"))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), rendered.width, rendered.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&pixels)?;
    writer.finish()?;
    source.manifest.diagnostics.extend(rendered.warnings.iter().map(|warning| format!("PDF rendering warning: {warning:?}")));
    let actual_dpi = f64::from(scale) * 72.0;
    if actual_dpi < dpi * 0.9999 {
        source
            .manifest
            .diagnostics
            .push(format!("Rendering resolution reduced from {dpi:.2} to {actual_dpi:.2} DPI to fit the renderer and pixel limits."));
    }
    let result = RenderManifest {
        document: source.manifest,
        width: rendered.width,
        height: rendered.height,
        dpi: actual_dpi,
        raster_to_page: raster_to_page(rotation, region, unit, f64::from(scale)),
        page_index,
        region,
    };
    fs::write(output.join("manifest.json"), serde_json::to_vec(&result)?)?;
    Ok(result)
}

pub fn verify_markup(base: &Path, edited: &Path, max_pages: usize) -> Result<Verification> {
    let base = load(read(base)?, max_pages)?;
    let edited = load(read(edited)?, max_pages)?;
    if base.pages.len() != edited.pages.len() || base.pages.iter().zip(&edited.pages).any(|(a, b)| a.obj != b.obj) {
        return Ok(Verification { verified: false, reason: "The PDF page identity or order changed. Select and align the plan again.".into() });
    }
    let compared =
        fingerprint::drawing(&base.document, &base.pages).and_then(|a| fingerprint::drawing(&edited.document, &edited.pages).map(|b| a == b));
    Ok(match compared {
        Ok(true) => Verification {
            verified: true,
            reason: "Page geometry, drawing content and rendering resources are unchanged; only permitted review markup or metadata changed.".into(),
        },
        Ok(false) => Verification {
            verified: false,
            reason: "Page geometry, drawing content or rendering resources changed. Select and align the plan again.".into(),
        },
        Err(_) => {
            Verification { verified: false, reason: "Unchanged drawing geometry could not be verified. Select and align the plan again.".into() }
        }
    })
}

pub fn extract(input: &Path, pages: &[usize], output: &Path, max_pages: usize) -> Result<Manifest> {
    let source = load(read(input)?, max_pages)?;
    if pages.is_empty()
        || pages.len() > source.pages.len()
        || pages.windows(2).any(|pair| pair[0] >= pair[1])
        || pages.iter().any(|page| *page >= source.pages.len())
    {
        return Err(invalid("Choose existing pages in document order without duplicates."));
    }
    let extracted = pdfcraft_organize::extract_pages(&source.document, pages)?;
    let bytes = pdfcraft_cos::write_full(&extracted, &SaveOptions::default())?;
    let result = load(bytes.clone(), max_pages)?.manifest;
    if output.exists() {
        return Err(invalid("The extraction output already exists. Use a new destination."));
    }
    let parent = output.parent().ok_or_else(|| invalid("The extraction needs an output directory."))?;
    fs::create_dir_all(parent)?;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(output)?;
    use std::io::Write;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(result)
}

#[cfg(test)]
mod tests;
