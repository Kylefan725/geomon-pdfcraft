use pdfcraft_engine::Edit;
use pdfcraft_testkit::Pdf;
use pdfcraft_ui_egui::{
    PdfCraftApp,
    hosted::{Context, Hosted, Inbox, Input, PROTOCOL_VERSION},
};
use serde_json::Value;
use std::{cell::RefCell, rc::Rc};

fn context() -> Context {
    Context {
        session_id: "00000000-0000-4000-8000-000000000001".into(),
        document_id: 1,
        revision_id: 2,
        version: 3,
    }
}
fn setup(editable: bool) -> (PdfCraftApp, Inbox, Rc<RefCell<Vec<Value>>>) {
    let mut app = PdfCraftApp::new();
    let inbox = Inbox::default();
    let out = Rc::new(RefCell::new(Vec::new()));
    let capture = out.clone();
    app.hosted = Some(Hosted::new(
        inbox.clone(),
        Box::new(move |value, _| capture.borrow_mut().push(value)),
    ));
    let bytes=Pdf::new().obj("<< /Type /Catalog /Pages 2 0 R >>").obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .obj("<< /Type /Page /Parent 2 0 R /MediaBox [20 10 220 150] /UserUnit 2 /Rotate 90 /Resources << >> /Contents 4 0 R >>")
        .stream("",b"20 10 100 100 re S").build();
    inbox.lock().unwrap().push(Input::Open {
        context: context(),
        name: "plan.pdf".into(),
        can_edit: editable,
        locale: "zh-HK".into(),
        theme: "light".into(),
        page_index: Some(0),
        region: None,
        bytes,
    });
    app.hosted_tick();
    (app, inbox, out)
}
#[test]
fn readonly_blocks_commands_and_edits() {
    let (mut app, _, _) = setup(false);
    for command in [
        "file.open",
        "file.save",
        "page.rotate",
        "page.delete",
        "edit.undo",
        "measure.distance",
        "ocr.recognize",
        "form.prepare",
        "edit.text",
        "secure.redact",
    ] {
        assert!(!app.execute(command), "{command}");
    }
    assert!(!app.apply_edit(Edit::RotatePages {
        pages: vec![0],
        degrees: 90
    }));
    assert!(app.first_dirty().is_none());
    assert!(!app.session.javascript());
    assert_eq!(app.views.len(), 1);
}
#[test]
fn manager_can_review_but_cannot_delete_or_change_content() {
    let (mut app, _, _) = setup(true);
    assert!(!app.apply_edit(Edit::DeletePages { pages: vec![0] }));
    assert!(!app.apply_edit(Edit::SetInfo {
        key: "Title".into(),
        value: "Changed".into()
    }));
    assert!(app.apply_edit(Edit::RotatePages {
        pages: vec![0],
        degrees: 90
    }));
    assert!(app.first_dirty().is_some());
}
#[test]
fn save_failure_retry_and_ack_keep_unsaved_work_until_stored() {
    let (mut app, inbox, out) = setup(true);
    assert!(app.apply_edit(Edit::RotatePages {
        pages: vec![0],
        degrees: 90
    }));
    app.hosted_save();
    let first = out
        .borrow()
        .iter()
        .find(|v| v["type"] == "geomon-pdf:save")
        .unwrap()
        .clone();
    let request = first["requestId"].as_str().unwrap().to_owned();
    assert!(app.first_dirty().is_some());
    assert!(!app.apply_edit(Edit::RotatePages {
        pages: vec![0],
        degrees: 90
    }));
    inbox.lock().unwrap().push(Input::SaveResult {
        context: context(),
        request_id: request.clone(),
        ok: false,
        error: Some("Network unavailable".into()),
        revision_id: None,
        version: None,
    });
    app.hosted_tick();
    assert!(app.first_dirty().is_some());
    app.hosted_save();
    let saves: Vec<_> = out
        .borrow()
        .iter()
        .filter(|v| v["type"] == "geomon-pdf:save")
        .cloned()
        .collect();
    assert_eq!(saves.len(), 2);
    assert_eq!(saves[0]["requestId"], saves[1]["requestId"]);
    let mut wrong = context();
    wrong.revision_id = 99;
    inbox.lock().unwrap().push(Input::SaveResult {
        context: wrong,
        request_id: request.clone(),
        ok: true,
        error: None,
        revision_id: Some(4),
        version: Some(1),
    });
    app.hosted_tick();
    assert!(app.first_dirty().is_some());
    inbox.lock().unwrap().push(Input::SaveResult {
        context: context(),
        request_id: request,
        ok: true,
        error: None,
        revision_id: Some(4),
        version: Some(1),
    });
    app.hosted_tick();
    assert!(app.first_dirty().is_none());
    let last = out.borrow().last().unwrap().clone();
    assert_eq!(last["context"]["revisionId"], 4);
    assert_eq!(last["dirty"], false);
}
fn of_type(out: &Rc<RefCell<Vec<Value>>>, kind: &str) -> Vec<Value> {
    out.borrow()
        .iter()
        .filter(|v| v["type"] == kind)
        .cloned()
        .collect()
}
#[test]
fn host_save_requests_reuse_one_attempt_per_edit_generation() {
    let (mut app, inbox, out) = setup(true);
    inbox
        .lock()
        .unwrap()
        .push(Input::RequestSave { context: context() });
    app.hosted_tick();
    assert!(of_type(&out, "geomon-pdf:save").is_empty());
    assert!(of_type(&out, "geomon-pdf:save-refused").is_empty());
    assert!(app.apply_edit(Edit::RotatePages {
        pages: vec![0],
        degrees: 90
    }));
    for _ in 0..2 {
        inbox
            .lock()
            .unwrap()
            .push(Input::RequestSave { context: context() });
    }
    app.hosted_tick();
    let saves = of_type(&out, "geomon-pdf:save");
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0]["protocolVersion"], PROTOCOL_VERSION);
    let request = saves[0]["requestId"].as_str().unwrap().to_owned();
    inbox
        .lock()
        .unwrap()
        .push(Input::RequestSave { context: context() });
    app.hosted_tick();
    inbox.lock().unwrap().push(Input::SaveResult {
        context: context(),
        request_id: request.clone(),
        ok: false,
        error: Some("Network unavailable".into()),
        revision_id: None,
        version: None,
    });
    app.hosted_tick();
    let mut stale = context();
    stale.revision_id = 99;
    inbox
        .lock()
        .unwrap()
        .push(Input::RequestSave { context: stale });
    app.hosted_tick();
    assert_eq!(of_type(&out, "geomon-pdf:save").len(), 2);
    inbox
        .lock()
        .unwrap()
        .push(Input::RequestSave { context: context() });
    app.hosted_tick();
    let saves = of_type(&out, "geomon-pdf:save");
    assert_eq!(saves.len(), 3);
    assert!(saves.iter().all(|save| save["requestId"] == request.as_str()));
    assert!(of_type(&out, "geomon-pdf:save-refused").is_empty());
}
#[test]
fn host_save_request_reports_why_the_viewer_refused() {
    let (mut app, inbox, out) = setup(true);
    app.views[0].comments.add_box = "開挖第一階段".into();
    app.hosted_tick();
    inbox
        .lock()
        .unwrap()
        .push(Input::RequestSave { context: context() });
    app.hosted_tick();
    assert!(of_type(&out, "geomon-pdf:save").is_empty());
    let refused = of_type(&out, "geomon-pdf:save-refused");
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0]["context"]["revisionId"], 2);
    assert_eq!(
        refused[0]["error"],
        "請先發佈或取消正在輸入的註解，再儲存草稿。您的文字仍保留在編輯器內。"
    );
    assert_eq!(app.views[0].comments.add_box, "開挖第一階段");
}
#[test]
fn configure_does_not_rebase_a_dirty_document() {
    let (mut app, inbox, out) = setup(true);
    assert!(app.apply_edit(Edit::RotatePages {
        pages: vec![0],
        degrees: 90
    }));
    inbox.lock().unwrap().push(Input::Configure {
        context: context(),
        can_edit: true,
        locale: "en".into(),
        theme: "dark".into(),
        next_version: Some(20),
    });
    app.hosted_tick();
    let ack = out
        .borrow()
        .iter()
        .find(|v| v["type"] == "geomon-pdf:configured")
        .unwrap()
        .clone();
    assert_eq!(ack["context"]["version"], 3);
    assert_eq!(app.language, "en");
}

#[test]
fn canonical_coordinates_ignore_zoom_and_view_rotation() {
    for page_rotation in [0u16, 90, 180, 270] {
        let (width, height) = if page_rotation % 180 == 0 {
            (400.0, 280.0)
        } else {
            (280.0, 400.0)
        };
        let info = pdfcraft_render::PageInfo {
            width,
            height,
            crop: [20.0, 10.0, 220.0, 150.0],
            rotation: page_rotation,
            label: "1".into(),
        };
        let view = info.user_to_view(57.0, 89.0);
        for zoom in [0.5, 1.0, 2.25, 4.0] {
            for rotation in [0u16, 90, 180, 270] {
                let (rw, rh) = if rotation % 180 == 0 {
                    (width * zoom, height * zoom)
                } else {
                    (height * zoom, width * zoom)
                };
                let rect = egui::Rect::from_min_size(egui::pos2(83.0, 147.0), egui::vec2(rw, rh));
                let (u, v) = (view[0] / width, view[1] / height);
                let (x, y) = match rotation {
                    90 => (1.0 - v, u),
                    180 => (1.0 - u, 1.0 - v),
                    270 => (v, 1.0 - u),
                    _ => (u, v),
                };
                let native = pdfcraft_ui_egui::hosted::page_point(
                    &info,
                    rect,
                    rotation,
                    egui::pos2(rect.left() + x * rw, rect.top() + y * rh),
                );
                assert!(
                    (native[0] - 57.0).abs() < 0.0001 && (native[1] - 89.0).abs() < 0.0001,
                    "{native:?}"
                );
            }
        }
    }
}

#[test]
fn unposted_review_text_cannot_be_silently_saved_or_replaced() {
    let (mut app, inbox, out) = setup(true);
    app.views[0].comments.add_box = "開挖第一階段".into();
    app.hosted_tick();
    assert!(app.first_dirty().is_some());
    app.hosted_save();
    assert!(out.borrow().iter().all(|m| m["type"] != "geomon-pdf:save"));
    inbox.lock().unwrap().push(Input::Configure {
        context: context(),
        can_edit: true,
        locale: "en".into(),
        theme: "light".into(),
        next_version: Some(9),
    });
    app.hosted_tick();
    assert_eq!(app.views[0].comments.add_box, "開挖第一階段");
    let ack = out
        .borrow()
        .iter()
        .rev()
        .find(|m| m["type"] == "geomon-pdf:configured")
        .unwrap()
        .clone();
    assert_eq!(ack["context"]["version"], 3);
    app.views[0].comments.add_box.clear();
    app.views[0].comments.editing = Some((0, 0, "Edited note awaiting Post".into()));
    assert!(app.first_dirty().is_some());
    app.hosted_save();
    assert!(out.borrow().iter().all(|m| m["type"] != "geomon-pdf:save"));
}

#[test]
fn note_and_text_box_composers_preserve_unposted_text() {
    use pdfcraft_ui_egui::comments::{Composer, ComposerKind};
    for kind in [
        ComposerKind::Note,
        ComposerKind::TextBox {
            size: Some([80.0, 30.0]),
        },
    ] {
        let (mut app, _, out) = setup(true);
        app.views[0].comments.composer = Some(Composer {
            page: 0,
            at: [30.0, 80.0],
            kind,
            text: "開挖註解".into(),
            focus: false,
        });
        app.hosted_tick();
        assert!(app.first_dirty().is_some());
        app.hosted_save();
        assert!(out.borrow().iter().all(|m| m["type"] != "geomon-pdf:save"));
        assert_eq!(
            app.views[0].comments.composer.as_ref().unwrap().text,
            "開挖註解"
        );
    }
}

#[test]
fn independent_opening_never_replaces_the_bound_document() {
    let (mut app, inbox, _) = setup(true);
    let other = Pdf::new()
        .obj("<< /Type /Catalog /Pages 2 0 R >>")
        .obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .obj("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>")
        .build();
    let original=Pdf::new()
        .obj("<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(embedded.pdf) 5 0 R] >> >> >>")
        .obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .obj("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R /Annots [7 0 R] >>")
        .stream("",b"10 10 30 40 re S")
        .obj("<< /Type /Filespec /F (embedded.pdf) /EF << /F 6 0 R >> >>")
        .stream("/Type /EmbeddedFile /Subtype /application#2Fpdf",&other)
        .obj("<< /Type /Annot /Subtype /FileAttachment /Rect [10 10 30 30] /FS 5 0 R /P 3 0 R >>").build();
    inbox.lock().unwrap().push(Input::Open {
        context: context(),
        name: "with-attachment.pdf".into(),
        can_edit: true,
        locale: "en".into(),
        theme: "light".into(),
        page_index: Some(0),
        region: None,
        bytes: original,
    });
    app.hosted_tick();
    let id = app.views[0].id;
    assert!(!app.session.get(id).unwrap().info.attachments.is_empty());
    app.attachment_action(id, 0, true);
    app.comment_attachment_action(id, 0, 0, true);
    assert!(app.open_bytes("attachment.pdf", None, other).is_err());
    assert_eq!(app.views.len(), 1);
    assert_eq!(app.views[0].id, id);
    assert_eq!(app.active, Some(0));
}
