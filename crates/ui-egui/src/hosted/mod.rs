//! GeoMon's single-document review surface and acknowledged revision saves.
use crate::{PdfCraftApp, QuickTool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
mod ui;
use pdfcraft_engine::{DocId, Edit};

pub const PROTOCOL_VERSION: u64 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub session_id: String,
    pub document_id: u64,
    pub revision_id: u64,
    pub version: u64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum Input {
    #[serde(rename = "geomon-pdf:open")]
    Open {
        #[serde(flatten)]
        context: Context,
        name: String,
        can_edit: bool,
        locale: String,
        theme: String,
        page_index: Option<usize>,
        region: Option<[f64; 4]>,
        #[serde(skip)]
        bytes: Vec<u8>,
    },
    #[serde(rename = "geomon-pdf:configure")]
    Configure { context: Context, can_edit: bool, locale: String, theme: String, next_version: Option<u64> },
    #[serde(rename = "geomon-pdf:request-save")]
    RequestSave { context: Context },
    #[serde(rename = "geomon-pdf:save-result")]
    SaveResult { context: Context, request_id: String, ok: bool, error: Option<String>, revision_id: Option<u64>, version: Option<u64> },
    #[serde(rename = "geomon-pdf:request-point")]
    RequestPoint { context: Context, request_id: String },
    #[serde(rename = "geomon-pdf:cancel-point")]
    CancelPoint { context: Context, request_id: String },
    #[serde(rename = "geomon-pdf:select-plan")]
    SelectPlan { context: Context, page_index: usize, region: Option<[f64; 4]> },
}

pub type Inbox = Arc<Mutex<Vec<Input>>>;
pub type Output = Box<dyn FnMut(Value, Option<Arc<Vec<u8>>>)>;
struct SavedAttempt {
    request_id: String,
    generation: u64,
    bytes: Arc<Vec<u8>>,
    doc: DocId,
}

pub struct Hosted {
    pub inbox: Inbox,
    output: Output,
    context: Option<Context>,
    can_edit: bool,
    pending: bool,
    attempt: Option<SavedAttempt>,
    request_sequence: u64,
    dirty_reported: Option<bool>,
    capture: Option<String>,
    selecting: bool,
    drag: Option<(usize, [f64; 2])>,
    selection: Option<(usize, [f64; 4])>,
    properties: bool,
    error: Option<String>,
}

impl Hosted {
    pub fn new(inbox: Inbox, output: Output) -> Self {
        Self {
            inbox,
            output,
            context: None,
            can_edit: false,
            pending: false,
            attempt: None,
            request_sequence: 0,
            dirty_reported: None,
            capture: None,
            selecting: false,
            drag: None,
            selection: None,
            properties: false,
            error: None,
        }
    }
    pub fn editable(&self) -> bool {
        self.can_edit && !self.pending && self.capture.is_none() && !self.selecting
    }
    pub fn capturing(&self) -> bool {
        self.capture.is_some() || self.selecting
    }
    fn matches(&self, context: &Context) -> bool {
        self.context.as_ref() == Some(context)
    }
    fn emit(&mut self, kind: &str, extra: Value, bytes: Option<Arc<Vec<u8>>>) {
        let Some(context) = self.context.as_ref() else {
            return;
        };
        let mut value = json!({
            "type": format!("geomon-pdf:{kind}"),
            "protocolVersion": PROTOCOL_VERSION,
            "context": context,
        });
        if let (Some(to), Some(from)) = (value.as_object_mut(), extra.as_object()) {
            to.extend(from.clone());
        }
        (self.output)(value, bytes);
    }
    fn send_save(&mut self) {
        if let Some(attempt) = &self.attempt {
            let (request, bytes) = (attempt.request_id.clone(), attempt.bytes.clone());
            self.pending = true;
            self.error = None;
            self.emit("save", json!({"requestId":request}), Some(bytes));
        }
    }
}

fn valid_region(region: [f64; 4]) -> bool {
    region.iter().all(|n| n.is_finite()) && region[2] > region[0] && region[3] > region[1]
}

pub fn page_point(info: &pdfcraft_render::PageInfo, rect: egui::Rect, view_rotation: u16, at: egui::Pos2) -> [f64; 2] {
    let xf = crate::canvas::PageXform { rect, rot: view_rotation, pw: info.width, ph: info.height };
    let (x, y) = xf.screen_to_view(at);
    info.view_to_user(x, y).map(f64::from)
}

pub fn permitted_edit(edit: &Edit) -> bool {
    if let Edit::AddAnnotation(annotation) = edit {
        use pdfcraft_engine::Shape;
        return matches!(
            annotation.shape,
            Shape::Note { .. }
                | Shape::TextMarkup { .. }
                | Shape::Rectangle { .. }
                | Shape::Oval { .. }
                | Shape::Line { .. }
                | Shape::Ink { .. }
                | Shape::TextBox { .. }
                | Shape::Polygon { .. }
                | Shape::PolyLine { .. }
                | Shape::Callout { .. }
        );
    }
    matches!(
        edit,
        Edit::RotatePages { .. }
            | Edit::SetPageBox { which: pdfcraft_engine::PageBox::Crop, .. }
            | Edit::DeleteAnnotation { .. }
            | Edit::SetAnnotationContents { .. }
            | Edit::ReplyToAnnotation { .. }
            | Edit::SetAnnotationStatus { .. }
            | Edit::EraseInk { .. }
            | Edit::MarkAnnotation { .. }
            | Edit::LockAnnotation { .. }
            | Edit::MoveAnnotation { .. }
            | Edit::ReshapeAnnotation { .. }
            | Edit::ResizeAnnotation { .. }
            | Edit::StyleAnnotation { .. }
            | Edit::FillAnnotation { .. }
            | Edit::FontSizeAnnotation { .. }
            | Edit::SetAnnotationInfo { .. }
    )
}

impl PdfCraftApp {
    pub(crate) fn hosted_review_draft(&self, index: usize) -> bool {
        let Some(view) = self.views.get(index) else {
            return false;
        };
        let comments = &view.comments;
        comments.composer.as_ref().is_some_and(|draft| !draft.text.trim().is_empty())
            || !comments.reply.trim().is_empty()
            || !comments.add_box.trim().is_empty()
            || comments.editing.as_ref().is_some_and(|(page, index, text)| {
                self.session
                    .get(view.id)
                    .and_then(|doc| doc.info.annotations.iter().find(|a| a.page == *page && a.index == *index))
                    .map_or(!text.is_empty(), |a| a.contents.as_deref().unwrap_or("") != text)
            })
    }
    pub(crate) fn hosted_dirty(&self) -> bool {
        self.first_dirty().is_some() || self.views.iter().enumerate().any(|(index, _)| self.hosted_review_draft(index))
    }
    pub fn hosted_command_allowed(&self, id: &str) -> bool {
        let Some(host) = &self.hosted else {
            return true;
        };
        if host.pending {
            return false;
        }
        matches!(
            id,
            "edit.find"
                | "edit.copy"
                | "edit.select_all"
                | "view.fit_width_scrolling"
                | "view.fit_one_page"
                | "view.previous_view"
                | "view.next_view"
                | "view.marquee_zoom"
        ) || host.editable() && matches!(id, "file.save" | "edit.undo" | "edit.redo" | "page.crop" | "page.rotate" | "page.rotate_ccw")
    }
    pub fn hosted_edit_allowed(&self, edit: &Edit) -> bool {
        let Some(host) = &self.hosted else {
            return true;
        };
        if !host.editable() || !permitted_edit(edit) {
            return false;
        }
        let target = match edit {
            Edit::DeleteAnnotation { page, index }
            | Edit::SetAnnotationContents { page, index, .. }
            | Edit::ReplyToAnnotation { page, index, .. }
            | Edit::SetAnnotationStatus { page, index, .. }
            | Edit::EraseInk { page, index, .. }
            | Edit::MarkAnnotation { page, index, .. }
            | Edit::LockAnnotation { page, index, .. }
            | Edit::MoveAnnotation { page, index, .. }
            | Edit::ReshapeAnnotation { page, index, .. }
            | Edit::ResizeAnnotation { page, index, .. }
            | Edit::StyleAnnotation { page, index, .. }
            | Edit::FillAnnotation { page, index, .. }
            | Edit::FontSizeAnnotation { page, index, .. }
            | Edit::SetAnnotationInfo { page, index, .. } => Some((*page, *index)),
            _ => None,
        };
        target.is_none_or(|(page, index)| {
            self.active_ids()
                .and_then(|(_, id)| self.session.get(id))
                .and_then(|doc| doc.info.annotations.iter().find(|a| a.page == page && a.index == index))
                .is_some_and(|a| {
                    !a.fill_sign
                        && matches!(
                            a.subtype.as_str(),
                            "Text"
                                | "FreeText"
                                | "Line"
                                | "Square"
                                | "Circle"
                                | "Polygon"
                                | "PolyLine"
                                | "Highlight"
                                | "Underline"
                                | "Squiggly"
                                | "StrikeOut"
                                | "Ink"
                        )
                })
        })
    }
    fn hosted_preferences(&mut self, locale: &str, theme: &str) {
        self.language = if locale == "zh-HK" { "zh-hant" } else { "en" }.into();
        self.set_theme_preference(if theme == "dark" { crate::theme::ThemePreference::Dark } else { crate::theme::ThemePreference::Light });
        self.session.set_javascript(false);
        self.recovery = None;
    }
    pub fn hosted_tick(&mut self) {
        let Some(mut host) = self.hosted.take() else {
            return;
        };
        let messages = host.inbox.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        let mut save_requested = false;
        for message in messages {
            match message {
                Input::RequestSave { context } if host.matches(&context) => {
                    if host.pending {
                        host.send_save();
                    } else {
                        save_requested = true;
                    }
                }
                Input::Open { context, name, can_edit, locale, theme, page_index, region, bytes } => {
                    if self.hosted_dirty() || host.pending {
                        continue;
                    }
                    if bytes.is_empty()
                        || bytes.len() > 100 * 1024 * 1024
                        || (context.session_id.len() != 36 || !context.session_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
                        || context.document_id == 0
                        || context.revision_id == 0
                    {
                        continue;
                    }
                    self.hosted_preferences(&locale, &theme);
                    let result = self.session.open(name, None, Arc::new(bytes), None);
                    host.context = Some(context);
                    host.can_edit = can_edit;
                    host.attempt = None;
                    host.error = None;
                    host.capture = None;
                    host.selecting = false;
                    host.dirty_reported = None;
                    match result {
                        Ok(id) => {
                            for view in self.views.drain(..) {
                                self.session.close(view.id);
                            }
                            if let Some(doc) = self.session.get(id) {
                                let mut view = crate::DocView::new(id, &doc.info, self.view_defaults);
                                view.hosted = true;
                                view.set_display(crate::canvas::PageLayout::Single, false);
                                view.set_fit(crate::canvas::Fit::Page);
                                view.current = page_index.unwrap_or(0).min(doc.info.pages.len().saturating_sub(1));
                                view.goto = Some((view.current, 0.0));
                                self.views.push(view);
                                self.active = Some(0);
                                self.mode = crate::Mode::Read;
                                self.quick_tool = QuickTool::Select;
                                self.left_open = false;
                                self.right = None;
                                host.selection = page_index.zip(region.filter(|r| valid_region(*r)));
                                host.emit("loaded", json!({}), None);
                            }
                        }
                        Err(error) => host.emit("failed", json!({"error":error.to_string()}), None),
                    }
                }
                Input::Configure { context, can_edit, locale, theme, next_version } if host.matches(&context) => {
                    self.hosted_preferences(&locale, &theme);
                    host.can_edit = can_edit;
                    if !self.hosted_dirty()
                        && !host.pending
                        && let Some(version) = next_version.filter(|v| *v >= context.version)
                        && let Some(current) = &mut host.context
                    {
                        current.version = version;
                    }
                    if !host.editable() {
                        self.quick_tool = QuickTool::Select;
                    }
                    host.emit("configured", json!({}), None);
                }
                Input::SaveResult { context, request_id, ok, error, revision_id, version } if host.matches(&context) => {
                    if !host.pending || host.attempt.as_ref().is_none_or(|a| a.request_id != request_id) {
                        continue;
                    }
                    if ok {
                        let (Some(revision_id), Some(version)) = (revision_id.filter(|v| *v > 0), version) else {
                            continue;
                        };
                        if let Some(attempt) = &host.attempt {
                            match self.session.mark_saved(attempt.doc, attempt.bytes.clone(), None) {
                                Ok(()) => {
                                    if let Some(doc) = self.session.get(attempt.doc)
                                        && let Some(view) = self.views.first_mut()
                                    {
                                        view.document_changed(&doc.info);
                                    }
                                    if let Some(current) = &mut host.context {
                                        current.revision_id = revision_id;
                                        current.version = version;
                                    }
                                    host.attempt = None;
                                    host.selection = None;
                                    host.capture = None;
                                    host.selecting = false;
                                    host.error = None;
                                    host.can_edit = false;
                                }
                                Err(error) => host.error = Some(error.to_string()),
                            }
                        }
                    } else {
                        host.error = Some(error.unwrap_or_else(|| "Save failed. Your unsaved changes are still here.".into()));
                    }
                    host.pending = false;
                }
                Input::RequestPoint { context, request_id } if host.matches(&context) && !host.pending && !self.hosted_dirty() => {
                    host.capture = Some(request_id);
                    host.selecting = false;
                    self.quick_tool = QuickTool::Select;
                }
                Input::CancelPoint { context, request_id } if host.matches(&context) && host.capture.as_ref() == Some(&request_id) => {
                    host.capture = None;
                }
                Input::SelectPlan { context, page_index, region } if host.matches(&context) && !host.pending && !self.hosted_dirty() => {
                    if let Some((i, id)) = self.active_ids()
                        && let Some(doc) = self.session.get(id)
                        && page_index < doc.info.pages.len()
                    {
                        self.views[i].current = page_index;
                        self.views[i].goto = Some((page_index, 0.0));
                        host.selection = region.filter(|r| valid_region(*r)).map(|r| (page_index, r));
                    }
                }
                _ => {}
            }
        }
        let dirty = self.hosted_dirty();
        self.unsaved_flag.store(dirty, std::sync::atomic::Ordering::Relaxed);
        if host.dirty_reported != Some(dirty) {
            host.dirty_reported = Some(dirty);
            host.emit("dirty", json!({"dirty":dirty}), None);
        }
        self.session.set_javascript(false);
        self.hosted = Some(host);
        if save_requested {
            self.hosted_request_save();
        }
    }
    fn hosted_request_save(&mut self) {
        if !self.hosted_dirty() {
            return;
        }
        if let Some(host) = &mut self.hosted {
            host.error = None;
        }
        self.hosted_save();
        let zh = self.language == "zh-hant";
        let Some(host) = self.hosted.as_mut() else {
            return;
        };
        if host.pending {
            return;
        }
        let error = host.error.clone().unwrap_or_else(|| {
            if zh {
                "目前無法儲存草稿。請先完成或取消檢視器內的操作。"
            } else {
                "The draft cannot be saved yet. Finish or cancel the current action in the viewer first."
            }
            .into()
        });
        host.emit("save-refused", json!({"error":error}), None);
    }
    pub fn hosted_save(&mut self) -> bool {
        if !self.hosted.as_ref().is_some_and(Hosted::editable) {
            return false;
        }
        let Some((_, id)) = self.active_ids() else {
            return false;
        };
        if self.active.is_some_and(|index| {
            self.hosted_review_draft(index) || self.views[index].comments.composer.is_some() || self.views[index].comments.editing.is_some()
        }) || self.dialog.is_some()
        {
            let message = if self.language == "zh-hant" {
                "請先發佈或取消正在輸入的註解，再儲存草稿。您的文字仍保留在編輯器內。"
            } else {
                "Post or cancel the comment you are typing before saving the draft. Your text is still in the editor."
            };
            if let Some(host) = &mut self.hosted {
                host.error = Some(message.into());
            }
            return false;
        }
        if !self.commit_typing_in(id) {
            return false;
        }
        let Some(doc) = self.session.get(id) else {
            return false;
        };
        let generation = doc.edit_generation();
        let bytes = match self.session.save_bytes(id) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.notify_error(error);
                return false;
            }
        };
        let Some(host) = self.hosted.as_mut() else {
            return false;
        };
        if !host.attempt.as_ref().is_some_and(|a| a.doc == id && a.generation == generation) {
            let Some(context) = &host.context else {
                return false;
            };
            host.request_sequence += 1;
            let request_id = format!("{}{:012x}", &context.session_id[..24], host.request_sequence);
            host.attempt = Some(SavedAttempt { request_id, generation, bytes, doc: id });
        }
        host.send_save();
        false
    }
}
