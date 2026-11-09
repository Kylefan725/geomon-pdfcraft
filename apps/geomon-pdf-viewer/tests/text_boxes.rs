use pdfcraft_engine::{Edit, NewAnnotation, Session, Shape, Style};
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};
use pdfcraft_testkit::Pdf;
use pdfcraft_ui_egui::comments::{drawn_text_box_rect, text_box_rect_on_page};
use std::sync::Arc;

#[test]
fn automatic_text_boxes_contain_rendered_latin_and_chinese_text() {
    let texts = [
        "Excavation stage one after strut installation",
        "開挖第一階段 — 支撐安裝後覆核",
        "Excavation 開挖第一階段 after strut installation. 支撐安裝後覆核，繼續檢查。\nStage two 第二階段。",
    ];
    for rotation in [0, 90] {
        for border in [1.0, 4.0] {
            for text in texts {
                let bytes = Pdf::new()
                    .obj("<< /Type /Catalog /Pages 2 0 R >>")
                    .obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
                    .obj(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 800 1000] /Rotate {rotation} /Resources << >> >>"))
                    .build();
                let mut session = Session::new();
                session.set_javascript(false);
                let id = session.open("note.pdf", None, Arc::new(bytes), None).unwrap();
                let rect = text_box_rect_on_page(&session.get(id).unwrap().info.pages[0], [400.0, 500.0], text, 12.0, border);
                session
                    .apply(
                        id,
                        Edit::AddAnnotation(NewAnnotation {
                            page: 0,
                            shape: Shape::TextBox { rect, font_size: 12.0 },
                            style: Style { width: border, ..Style::default() },
                            contents: text.into(),
                            author: "Engineer".into(),
                        }),
                    )
                    .unwrap();
                let info = &session.get(id).unwrap().info.pages[0];
                let a = info.user_to_view(rect[0] as f32, rect[1] as f32);
                let b = info.user_to_view(rect[2] as f32, rect[3] as f32);
                let bounds = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])];
                let mut renderer = PageRenderer::new(session.save_bytes(id).unwrap(), RenderConfig { transparent: true, ..RenderConfig::default() });
                let layer = renderer.render(RenderRequest { page: 0, kind: RequestKind::Text, ..RenderRequest::default() });
                assert!(layer.error.is_none(), "{:?}", layer.error);
                let layer = layer.text.unwrap();
                let expected: String = text.split_whitespace().collect();
                let actual: String = layer.plain_text().split_whitespace().collect();
                assert_eq!(actual, expected, "The complete annotation text must be rendered");
                for glyph in &layer.glyphs {
                    let g = glyph.rect;
                    assert!(
                        g[0] >= bounds[0] - 0.1 && g[1] >= bounds[1] - 0.1 && g[2] <= bounds[2] + 0.1 && g[3] <= bounds[3] + 0.1,
                        "Clipped {:?} at {g:?}, note {bounds:?}, rotation {rotation}, border {border}",
                        glyph.text
                    );
                }
                let pixels = renderer.render(RenderRequest { page: 0, scale: 0.5, ..RenderRequest::default() });
                assert!(pixels.error.is_none(), "{:?}", pixels.error);
                assert!(pixels.rgba.as_chunks::<4>().0.iter().any(|[_, _, _, alpha]| *alpha > 0));
            }
        }
    }
}

#[test]
fn a_deliberately_drawn_box_keeps_its_requested_size() {
    let page = pdfcraft_render::PageInfo { width: 300.0, height: 200.0, label: "1".into(), crop: [0.0, 0.0, 300.0, 200.0], rotation: 0 };
    let rect = drawn_text_box_rect(&page, [10.0, 190.0], [100.0, 100.0], "開挖 Stage one", 12.0, 1.0);
    for (actual, expected) in rect.into_iter().zip([10.0, 90.0, 110.0, 190.0]) {
        assert!((actual - expected).abs() < 0.00001);
    }
}

#[test]
fn a_click_note_wraps_inside_a_small_page_without_moving_its_anchor() {
    let text = "開挖第一階段 — 支撐安裝後覆核";
    for unit in [1, 2] {
        let bytes = Pdf::new()
            .obj("<< /Type /Catalog /Pages 2 0 R >>")
            .obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
            .obj(format!("<< /Type /Page /Parent 2 0 R /MediaBox [20 10 220 150] /UserUnit {unit} /Resources << >> >>"))
            .build();
        let mut session = Session::new();
        session.set_javascript(false);
        let id = session.open("small-plan.pdf", None, Arc::new(bytes), None).unwrap();
        let at = [86.22272491455078, 94.22502136230469];
        let page = session.get(id).unwrap().info.pages[0].clone();
        let rect = text_box_rect_on_page(&page, at, text, 12.0, 1.0);
        assert_eq!(rect[0], at[0]);
        assert_eq!(rect[3], at[1]);
        assert!(rect[2] <= 220.00001 && rect[1] >= 10.0);
        session
            .apply(
                id,
                Edit::AddAnnotation(NewAnnotation {
                    page: 0,
                    shape: Shape::TextBox { rect, font_size: 12.0 },
                    style: Style::default(),
                    contents: text.into(),
                    author: "Engineer".into(),
                }),
            )
            .unwrap();
        let mut renderer = PageRenderer::new(session.save_bytes(id).unwrap(), RenderConfig::default());
        let layer = renderer.render(RenderRequest { page: 0, kind: RequestKind::Text, ..RenderRequest::default() }).text.unwrap();
        assert_eq!(layer.plain_text().split_whitespace().collect::<String>(), text.split_whitespace().collect::<String>());
        let a = page.user_to_view(rect[0] as f32, rect[1] as f32);
        let b = page.user_to_view(rect[2] as f32, rect[3] as f32);
        for glyph in &layer.glyphs {
            assert!(
                glyph.rect[0] >= a[0] - 0.1 && glyph.rect[1] >= b[1] - 0.1 && glyph.rect[2] <= b[0] + 0.1 && glyph.rect[3] <= a[1] + 0.1,
                "Text clipped inside the automatic note: {:?}",
                glyph.text
            );
        }
        assert!(renderer.render(RenderRequest { page: 0, scale: 1.0, ..RenderRequest::default() }).error.is_none());
    }
}
