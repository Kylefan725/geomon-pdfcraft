#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use pdfcraft_annot::{Meta, NewAnnotation, NoteIcon, Shape, Style};
use pdfcraft_cos::{Dict, ObjRef, Stream};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "geomon-pdf-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< /Length {} {dict} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

fn pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}

fn drawing(rotation: i64, unit: f64, scanned: bool) -> Vec<u8> {
    let resources = if scanned {
        "/XObject << /Scan 5 0 R >>"
    } else {
        ""
    };
    let content = if scanned {
        b"q 20 0 0 20 40 20 cm /Scan Do Q".as_slice()
    } else {
        b"q 1 0 0 rg 40 20 20 20 re f Q".as_slice()
    };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [10 -20 250 180] /CropBox [20 10 220 150] /Rotate {rotation} /UserUnit {unit:.1} /Resources << {resources} >> /Contents 4 0 R >>").into_bytes(),
        stream("", content),
    ];
    if scanned {
        objects.push(stream("/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", &[255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0]));
    }
    pdf(objects)
}

fn multipage() -> Vec<u8> {
    pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 2 /Kids [3 0 R 5 0 R] /MediaBox [0 0 200 100] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 4 0 R >>".to_vec(),
        stream("", b"0 0 1 rg 20 20 20 20 re f"),
        b"<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 6 0 R >>".to_vec(),
        stream("", b"1 0 0 rg 40 20 20 20 re f"),
    ])
}

fn changed(bytes: &[u8], change: impl FnOnce(&mut Document)) -> Vec<u8> {
    let mut document = Document::open(Arc::new(bytes.to_vec())).unwrap();
    change(&mut document);
    pdfcraft_cos::write_incremental(&document, &SaveOptions::default()).unwrap()
}

fn transform(matrix: [f64; 6], x: f64, y: f64) -> [f64; 2] {
    let [a, b, c, d, e, f] = matrix;
    [a * x + c * y + e, b * x + d * y + f]
}

fn sample(png: &Path, matrix: [f64; 6], point: [f64; 2]) -> [u8; 4] {
    let [a, b, c, d, e, f] = matrix;
    let det = a * d - b * c;
    let x = ((d * (point[0] - e) - c * (point[1] - f)) / det).floor() as usize;
    let y = ((-b * (point[0] - e) + a * (point[1] - f)) / det).floor() as usize;
    let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(png).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let offset = (y * info.width as usize + x) * 4;
    pixels[offset..offset + 4].try_into().unwrap()
}

#[test]
fn inspection_preserves_native_boxes_rotation_and_user_unit() {
    let fixture = Fixture::new();
    let manifest = inspect(&fixture.write("page.pdf", &drawing(90, 2.0, false)), 100).unwrap();
    assert_eq!(manifest.page_count, 1);
    assert_eq!(manifest.pages[0].media_box, [10.0, -20.0, 250.0, 180.0]);
    assert_eq!(manifest.pages[0].crop_box, [20.0, 10.0, 220.0, 150.0]);
    assert_eq!(manifest.pages[0].rotation, 90);
    assert_eq!(manifest.pages[0].user_unit, 2.0);
    assert_eq!(
        (manifest.pages[0].width, manifest.pages[0].height),
        (280.0, 400.0)
    );
    assert_eq!(manifest.source_sha256.len(), 64);
}

#[test]
fn rendered_pixels_match_native_locations_for_rotations_and_resolutions() {
    let fixture = Fixture::new();
    for scanned in [false, true] {
        for rotation in [0, 90, 180, 270] {
            for dpi in [72.0, 144.0, 300.0] {
                let path = fixture.write("page.pdf", &drawing(rotation, 2.0, scanned));
                let out = fixture.0.join("render");
                let result = render(&path, 0, None, dpi, 1_000_000, &out, 100).unwrap();
                assert_eq!(result.region, [20.0, 10.0, 220.0, 150.0]);
                assert!(result.dpi <= dpi * 1.000001);
                let red = sample(
                    &out.join("reference.png"),
                    result.raster_to_page,
                    [50.0, 30.0],
                );
                assert!(
                    red[0] > 240 && red[1] < 10 && red[3] > 240,
                    "rotation {rotation}, dpi {dpi}, scan {scanned}: {red:?}"
                );
                let empty = sample(
                    &out.join("reference.png"),
                    result.raster_to_page,
                    [100.0, 100.0],
                );
                assert_eq!(empty[3], 0);
                let first_centre = transform(result.raster_to_page, 0.5, 0.5);
                assert!(first_centre.iter().all(|v| v.is_finite()));
            }
        }
    }
}

#[test]
fn selecting_regions_preserves_original_and_limits_pixels() {
    let fixture = Fixture::new();
    let bytes = drawing(270, 2.0, false);
    let path = fixture.write("page.pdf", &bytes);
    let region = [30.0, 15.0, 80.0, 55.0];
    let out = fixture.0.join("region");
    let result = render(&path, 0, Some(region), 300.0, 1000, &out, 100).unwrap();
    assert!(u64::from(result.width) * u64::from(result.height) <= 1000);
    assert!(result.dpi < 300.0);
    assert_eq!(result.region, region);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let red = sample(
        &out.join("reference.png"),
        result.raster_to_page,
        [50.0, 30.0],
    );
    assert!(red[0] > 230 && red[3] > 230);
    assert!(
        render(
            &path,
            0,
            Some([0.0, 0.0, 500.0, 500.0]),
            300.0,
            1000,
            &out,
            100
        )
        .is_err()
    );
}

#[test]
fn real_pdfcraft_markup_is_verified_and_rendered() {
    let fixture = Fixture::new();
    let bytes = drawing(90, 2.0, false);
    let base = fixture.write("original.pdf", &bytes);
    let annotated = changed(&bytes, |doc| {
        pdfcraft_annot::add_annotation(
            doc,
            &NewAnnotation {
                page: 0,
                shape: Shape::Note {
                    at: [120.0, 100.0],
                    icon: NoteIcon::Comment,
                },
                style: Style::default(),
                contents: "Construction review 現場核對".into(),
                author: "Engineer".into(),
            },
            &Meta {
                date: Some("D:20261101120000+08'00'".into()),
                id: "test-note".into(),
            },
        )
        .unwrap();
    });
    let edit = fixture.write("annotated.pdf", &annotated);
    assert!(verify_markup(&base, &edit, 100).unwrap().verified);
    assert_eq!(fs::read(base).unwrap(), bytes);
    let output = fixture.0.join("annotated");
    render(&edit, 0, None, 72.0, 1_000_000, &output, 100).unwrap();
    assert!(output.join("reference.png").is_file());
}

#[test]
fn chinese_text_annotations_keep_the_drawing_fingerprint() {
    let fixture = Fixture::new();
    let bytes = drawing(0, 1.0, false);
    let base = fixture.write("original.pdf", &bytes);
    let annotated = changed(&bytes, |doc| {
        pdfcraft_annot::add_annotation(
            doc,
            &NewAnnotation {
                page: 0,
                shape: Shape::TextBox {
                    rect: [30.0, 70.0, 190.0, 110.0],
                    font_size: 12.0,
                },
                style: Style::default(),
                contents: "開挖第一階段".into(),
                author: "工程師".into(),
            },
            &Meta {
                date: None,
                id: "chinese-note".into(),
            },
        )
        .unwrap();
    });
    let edit = fixture.write("chinese.pdf", &annotated);
    assert!(verify_markup(&base, &edit, 100).unwrap().verified);
    let mut renderer = PageRenderer::new(Arc::new(annotated), RenderConfig::default());
    let page = renderer.render(RenderRequest {
        page: 0,
        kind: pdfcraft_render::RequestKind::Text,
        ..RenderRequest::default()
    });
    assert!(page.error.is_none(), "{:?}", page.error);
    let text = page.text.unwrap().plain_text();
    assert!(
        text.contains("開挖第一階段"),
        "Chinese annotation text lost: {text}"
    );
}

#[test]
fn chinese_notes_can_be_edited_and_restyled_without_duplicate_font_programs() {
    let fixture = Fixture::new();
    let bytes = drawing(90, 2.0, false);
    let base = fixture.write("original.pdf", &bytes);
    let first = changed(&bytes, |doc| {
        pdfcraft_annot::add_annotation(
            doc,
            &NewAnnotation {
                page: 0,
                shape: Shape::TextBox {
                    rect: [30.0, 70.0, 190.0, 110.0],
                    font_size: 12.0,
                },
                style: Style::default(),
                contents: "開挖第一階段".into(),
                author: "工程師".into(),
            },
            &Meta {
                date: None,
                id: "chinese-first".into(),
            },
        )
        .unwrap();
    });
    let updated = changed(&first, |doc| {
        let meta = Meta {
            date: None,
            id: "chinese-updated".into(),
        };
        pdfcraft_annot::set_contents(doc, 0, 0, "開挖第二階段", &meta).unwrap();
        pdfcraft_annot::move_annotation(doc, 0, 0, 3.0, 2.0, &meta).unwrap();
        pdfcraft_annot::set_font_size(doc, 0, 0, 14.0, &meta).unwrap();
        pdfcraft_annot::add_annotation(
            doc,
            &NewAnnotation {
                page: 0,
                shape: Shape::TextBox {
                    rect: [30.0, 20.0, 190.0, 60.0],
                    font_size: 12.0,
                },
                style: Style::default(),
                contents: "支撐安裝".into(),
                author: "工程師".into(),
            },
            &Meta {
                date: None,
                id: "chinese-second".into(),
            },
        )
        .unwrap();
    });
    assert!(
        updated.len() < first.len() * 18 / 10,
        "The font program was embedded again for a second note"
    );
    let edit = fixture.write("updated.pdf", &updated);
    assert!(verify_markup(&base, &edit, 100).unwrap().verified);
    let loaded = load(updated.clone(), 100).unwrap();
    let annots = loaded
        .document
        .resolve(loaded.pages[0].dict.get(b"Annots").unwrap());
    let first_note = loaded
        .document
        .dict(&annots.as_array().unwrap()[0])
        .unwrap();
    assert_eq!(
        first_note
            .get(b"Contents")
            .unwrap()
            .as_string()
            .unwrap()
            .to_text(),
        "開挖第二階段"
    );
    if let Some(output) = std::env::var_os("CONSTRUCTION_PDF_TEST_ARTIFACTS") {
        let output = PathBuf::from(output);
        fs::create_dir_all(&output).unwrap();
        let source = output.join("chinese-notes.pdf");
        fs::write(&source, &updated).unwrap();
        render(&source, 0, None, 144.0, 1_000_000, &output, 100).unwrap();
    }
    let mut renderer = PageRenderer::new(Arc::new(updated), RenderConfig::default());
    let page = renderer.render(RenderRequest {
        page: 0,
        kind: pdfcraft_render::RequestKind::Text,
        ..RenderRequest::default()
    });
    assert!(page.error.is_none(), "{:?}", page.error);
    let text: String = page.text.unwrap().plain_text().split_whitespace().collect();
    assert!(
        text.contains("開挖第二階段") && text.contains("支撐安裝"),
        "Chinese note edits lost: {text}"
    );
}

#[test]
fn drawing_content_resources_and_page_changes_invalidate_alignment() {
    let fixture = Fixture::new();
    let bytes = drawing(0, 1.0, false);
    let base = fixture.write("original.pdf", &bytes);
    let changes: Vec<Box<dyn FnOnce(&mut Document)>> = vec![
        Box::new(|doc| {
            doc.update_dict(ObjRef::new(3, 0), |d| {
                d.set(b"Rotate".to_vec(), Object::Int(90))
            })
            .unwrap();
        }),
        Box::new(|doc| {
            doc.update_dict(ObjRef::new(3, 0), |d| {
                d.set(b"UserUnit".to_vec(), Object::Int(2))
            })
            .unwrap();
        }),
        Box::new(|doc| {
            doc.update_dict(ObjRef::new(3, 0), |d| {
                d.set(
                    b"CropBox".to_vec(),
                    Object::Array([20, 10, 200, 140].into_iter().map(Object::Int).collect()),
                )
            })
            .unwrap();
        }),
        Box::new(|doc| {
            doc.set(
                ObjRef::new(4, 0),
                Object::Stream(Stream::from_raw(
                    Dict::new(),
                    b"0 1 0 rg 40 20 20 20 re f".to_vec(),
                )),
            );
        }),
        Box::new(|doc| {
            doc.update_dict(ObjRef::new(3, 0), |d| {
                let mut resources = Dict::new();
                resources.set(
                    b"ProcSet".to_vec(),
                    Object::Array(vec![Object::Name(b"PDF".to_vec())]),
                );
                d.set(b"Resources".to_vec(), Object::Dict(resources));
            })
            .unwrap();
        }),
    ];
    for change in changes {
        let edited = fixture.write("changed.pdf", &changed(&bytes, change));
        assert!(!verify_markup(&base, &edited, 100).unwrap().verified);
    }
}

#[test]
fn page_order_and_extraction_are_explicit_and_source_is_preserved() {
    let fixture = Fixture::new();
    let bytes = multipage();
    let source = fixture.write("multi.pdf", &bytes);
    assert_eq!(inspect(&source, 100).unwrap().page_count, 2);
    assert!(inspect(&source, 1).is_err());
    let output = fixture.0.join("extracted.pdf");
    let extracted = extract(&source, &[1], &output, 100).unwrap();
    assert_eq!(extracted.page_count, 1);
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert!(!verify_markup(&source, &output, 100).unwrap().verified);
    assert!(extract(&source, &[1, 0], &fixture.0.join("reverse.pdf"), 100).is_err());
    assert!(extract(&source, &[1, 1], &fixture.0.join("duplicate.pdf"), 100).is_err());
    assert!(extract(&source, &[2], &fixture.0.join("missing.pdf"), 100).is_err());
    assert!(extract(&source, &[0], &output, 100).is_err());
    let reversed = changed(&bytes, |doc| {
        doc.update_dict(ObjRef::new(2, 0), |dict| {
            dict.set(
                b"Kids".to_vec(),
                Object::Array(vec![
                    Object::Ref(ObjRef::new(5, 0)),
                    Object::Ref(ObjRef::new(3, 0)),
                ]),
            )
        })
        .unwrap();
    });
    assert!(
        !verify_markup(&source, &fixture.write("reversed.pdf", &reversed), 100)
            .unwrap()
            .verified
    );
}

#[test]
fn invalid_and_active_pdfs_have_actionable_errors() {
    let fixture = Fixture::new();
    for bytes in [
        b"not a PDF".to_vec(),
        b"%PDF-1.7\ninvalid\n".to_vec(),
        drawing(13, 1.0, false),
        drawing(0, 0.5, false),
    ] {
        assert!(inspect(&fixture.write("bad.pdf", &bytes), 100).is_err());
    }
    let script = changed(&drawing(0, 1.0, false), |doc| {
        doc.update_dict(ObjRef::new(1, 0), |dict| {
            let mut action = Dict::new();
            action.set(b"S".to_vec(), Object::Name(b"JavaScript".to_vec()));
            action.set(
                b"JS".to_vec(),
                Object::String(pdfcraft_cos::PdfString::literal(
                    b"app.alert('hello')".to_vec(),
                )),
            );
            dict.set(b"OpenAction".to_vec(), Object::Dict(action));
        })
        .unwrap();
    });
    assert!(
        inspect(&fixture.write("active.pdf", &script), 100)
            .unwrap_err()
            .to_string()
            .contains("static")
    );
}

#[test]
fn encrypted_sources_are_rejected_even_when_the_open_password_is_empty() {
    let fixture = Fixture::new();
    for password in ["", "drawing-password"] {
        let mut doc = Document::open(Arc::new(drawing(0, 1.0, false))).unwrap();
        doc.set_encryption(&pdfcraft_crypt::NewEncryption {
            algorithm: pdfcraft_crypt::Algorithm::Aes256,
            user_password: password,
            owner_password: "owner-password",
            permissions: -4,
            encrypt_metadata: true,
            seed: [7; 32],
        })
        .unwrap();
        let bytes = pdfcraft_cos::write_full(&doc, &SaveOptions::default()).unwrap();
        let path = fixture.write("encrypted.pdf", &bytes);
        assert!(inspect(&path, 100).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn a_changed_scanned_image_resource_cannot_inherit_alignment() {
    let fixture = Fixture::new();
    let bytes = drawing(0, 1.0, true);
    let base = fixture.write("scan.pdf", &bytes);
    let changed_scan = changed(&bytes, |doc| {
        let image = doc.get(ObjRef::new(5, 0));
        let Object::Stream(image) = image.as_ref() else {
            panic!("image fixture is missing");
        };
        doc.set(
            ObjRef::new(5, 0),
            Object::Stream(Stream::from_raw(
                image.dict.clone(),
                vec![0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255],
            )),
        );
    });
    assert!(
        !verify_markup(
            &base,
            &fixture.write("changed-scan.pdf", &changed_scan),
            100
        )
        .unwrap()
        .verified
    );
}

#[test]
fn equivalent_integer_and_real_geometry_is_verified_without_rounding_large_integers() {
    let fixture = Fixture::new();
    let bytes = drawing(0, 1.0, false);
    let base = fixture.write("real.pdf", &bytes);
    let integer = changed(&bytes, |doc| {
        doc.update_dict(ObjRef::new(3, 0), |d| {
            d.set(b"UserUnit".to_vec(), Object::Int(1))
        })
        .unwrap();
    });
    assert!(
        verify_markup(&base, &fixture.write("integer.pdf", &integer), 100)
            .unwrap()
            .verified
    );
    let different = changed(&bytes, |doc| {
        doc.update_dict(ObjRef::new(3, 0), |d| {
            d.set(b"UserUnit".to_vec(), Object::Real(1.0001))
        })
        .unwrap();
    });
    assert!(
        !verify_markup(&base, &fixture.write("different.pdf", &different), 100)
            .unwrap()
            .verified
    );
    let high = changed(&bytes, |doc| {
        doc.update_dict(ObjRef::new(1, 0), |d| {
            d.set(b"TestNumber".to_vec(), Object::Int(9_007_199_254_740_993))
        })
        .unwrap();
    });
    let lower = changed(&bytes, |doc| {
        doc.update_dict(ObjRef::new(1, 0), |d| {
            d.set(b"TestNumber".to_vec(), Object::Int(9_007_199_254_740_992))
        })
        .unwrap();
    });
    assert!(
        !verify_markup(
            &fixture.write("high.pdf", &high),
            &fixture.write("lower.pdf", &lower),
            100
        )
        .unwrap()
        .verified
    );
}

#[test]
fn engine_session_markup_save_preserves_alignment_after_numeric_serialization() {
    let fixture = Fixture::new();
    let bytes = drawing(0, 1.0, false);
    let base = fixture.write("session-original.pdf", &bytes);
    let mut session = pdfcraft_engine::Session::new();
    session.set_javascript(false);
    let id = session
        .open("construction.pdf", None, Arc::new(bytes), None)
        .unwrap();
    session
        .apply(
            id,
            pdfcraft_engine::Edit::AddAnnotation(NewAnnotation {
                page: 0,
                shape: Shape::TextBox {
                    rect: [30.0, 70.0, 190.0, 110.0],
                    font_size: 12.0,
                },
                style: Style::default(),
                contents: "開挖第一階段 — 支撐安裝後覆核".into(),
                author: "工程師".into(),
            }),
        )
        .unwrap();
    session
        .apply(
            id,
            pdfcraft_engine::Edit::ResizeAnnotation {
                page: 0,
                index: 0,
                rect: [30.0, 60.0, 200.0, 115.0],
            },
        )
        .unwrap();
    let saved = session.save_bytes(id).unwrap();
    let edit = fixture.write("session-saved.pdf", &saved);
    assert!(verify_markup(&base, &edit, 100).unwrap().verified);
    session.mark_saved(id, saved, None).unwrap();
    session
        .apply(
            id,
            pdfcraft_engine::Edit::RotatePages {
                pages: vec![0],
                degrees: 90,
            },
        )
        .unwrap();
    let rotated = fixture.write("session-rotated.pdf", &session.save_bytes(id).unwrap());
    assert!(!verify_markup(&edit, &rotated, 100).unwrap().verified);
}
