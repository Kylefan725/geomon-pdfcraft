use super::{Hosted, page_point, valid_region};
use crate::{PdfCraftApp, QuickTool, RightPanel, comments::CommentTool};
use serde_json::json;

impl PdfCraftApp {
    pub fn hosted_ui(&mut self, ui: &mut egui::Ui) {
        let zh = self.language == "zh-hant";
        let label = |en: &'static str, tc: &'static str| if zh { tc } else { en };
        let editable = self.hosted.as_ref().is_some_and(Hosted::editable);
        let pending = self.hosted.as_ref().is_some_and(|h| h.pending);
        let dirty = self.hosted_dirty();
        let colors = crate::theme::Tokens::get(ui.ctx());
        let wide = ui.available_width() >= 720.0;
        let chrome = || egui::Frame::new().fill(colors.card).inner_margin(egui::Margin::symmetric(8, 6));
        egui::Panel::top("geomon_pdf_tools").frame(chrome()).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!pending, |ui| {
                    if crate::widgets::icons::button(ui, "zoom-out", 30.0, false, label("Zoom out", "縮小")).clicked()
                        && let Some(view) = self.active.and_then(|i| self.views.get_mut(i))
                    {
                        view.set_zoom((view.zoom / 1.25).max(0.05));
                    }
                    if wide && let Some(view) = self.active.and_then(|i| self.views.get(i)) {
                        ui.label(egui::RichText::new(format!("{:.0}%", view.zoom * 100.0)).small().color(colors.text_muted));
                    }
                    if crate::widgets::icons::button(ui, "zoom-in", 30.0, false, label("Zoom in", "放大")).clicked()
                        && let Some(view) = self.active.and_then(|i| self.views.get_mut(i))
                    {
                        view.set_zoom((view.zoom * 1.25).min(16.0));
                    }
                    if crate::widgets::icons::button(ui, "maximize", 30.0, false, label("Fit page", "符合頁面")).clicked() {
                        self.execute("view.fit_one_page");
                    }
                    ui.separator();
                    if let Some((i, id)) = self.active_ids()
                        && let Some(doc) = self.session.get(id)
                    {
                        let count = doc.info.pages.len();
                        let mut page = self.views[i].current + 1;
                        if ui.add(egui::DragValue::new(&mut page).range(1..=count)).on_hover_text(label("Go to page", "前往頁面")).changed() {
                            self.views[i].current = page - 1;
                            self.views[i].goto = Some((page - 1, 0.0));
                        }
                        ui.label(egui::RichText::new(format!("/ {count}")).small().color(colors.text_muted));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if pending {
                        ui.spinner();
                    } else if !self.hosted.as_ref().is_some_and(|host| host.can_edit) {
                        ui.label(egui::RichText::new(label("Read only", "唯讀")).small().color(colors.text_muted));
                    }
                    let more = crate::widgets::icons::button(ui, "ellipsis", 30.0, false, label("Page and view options", "頁面與檢視選項"));
                    egui::Popup::menu(&more).show(|ui| {
                        ui.set_min_width(190.0);
                        ui.label(egui::RichText::new(label("View", "檢視")).small().color(colors.text_muted));
                        for (panel, en, tc) in [(RightPanel::Comments, "Comments", "註解"), (RightPanel::Layers, "Layers", "圖層")] {
                            if ui.selectable_label(self.right == Some(panel), label(en, tc)).clicked() {
                                self.right = if self.right == Some(panel) { None } else { Some(panel) };
                                ui.close();
                            }
                        }
                        if ui.button(label("Rotate view", "旋轉檢視")).clicked() {
                            if let Some(view) = self.active.and_then(|i| self.views.get_mut(i)) {
                                view.rotation = (view.rotation + 90) % 360;
                            }
                            ui.close();
                        }
                        if ui.button(label("Document properties", "文件屬性")).clicked() {
                            if let Some(host) = &mut self.hosted {
                                host.properties = true;
                            }
                            ui.close();
                        }
                        ui.separator();
                        ui.add_enabled_ui(editable, |ui| {
                            ui.label(egui::RichText::new(label("Edit PDF page", "編輯 PDF 頁面")).small().color(colors.text_muted));
                            for (command, en, tc) in [
                                ("page.crop", "Crop PDF page", "裁剪 PDF 頁面"),
                                ("page.rotate", "Rotate PDF page", "旋轉 PDF 頁面"),
                                ("edit.undo", "Undo", "復原"),
                                ("edit.redo", "Redo", "重做"),
                            ] {
                                if ui.button(label(en, tc)).clicked() {
                                    self.execute(command);
                                    ui.close();
                                }
                            }
                        });
                    });
                    if crate::widgets::icons::button(ui, "search", 30.0, false, label("Search existing text", "搜尋現有文字")).clicked() {
                        self.execute("edit.find");
                    }
                    if wide {
                        for (panel, icon, en, tc) in
                            [(RightPanel::Layers, "layers", "Layers", "圖層"), (RightPanel::Comments, "message-square", "Comments", "註解")]
                        {
                            if crate::widgets::icons::button(ui, icon, 30.0, self.right == Some(panel), label(en, tc)).clicked() {
                                self.right = if self.right == Some(panel) { None } else { Some(panel) };
                            }
                        }
                    }
                });
            });
        });
        egui::Panel::bottom("geomon_pdf_status").frame(chrome()).show(ui, |ui| {
            if let Some(host) = &mut self.hosted {
                if let Some(error) = &host.error {
                    ui.colored_label(egui::Color32::from_rgb(190, 45, 45), error);
                }
                if host.capturing() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(if host.selecting {
                                label("Drag around the plan. The PDF stays unchanged.", "拖曳選取圖則範圍，PDF 內容不會改動。")
                            } else {
                                label("Click the reference point on the drawing.", "在圖則上點選參考點。")
                            })
                            .small(),
                        );
                        if ui.button(label("Cancel", "取消")).clicked() {
                            if let Some(request) = host.capture.take() {
                                host.emit("point-cancelled", json!({"requestId":request}), None);
                            }
                            host.selecting = false;
                            host.drag = None;
                        }
                    });
                } else {
                    let tool = match self.quick_tool {
                        QuickTool::Hand => label("Pan", "平移"),
                        QuickTool::Crop => label("Crop PDF page", "裁剪 PDF 頁面"),
                        QuickTool::Comment(tool) => crate::i18n::tr(crate::i18n::Lang::from_pref(&self.language), tool.label()),
                        _ => label("Select", "選取"),
                    };
                    ui.label(egui::RichText::new(tool).small().color(colors.text_muted));
                }
            }
        });
        egui::Panel::left("geomon_pdf_tool_rail")
            .resizable(false)
            .default_size(46.0)
            .frame(egui::Frame::new().fill(colors.card).inner_margin(egui::Margin::same(5)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.add_enabled_ui(!pending, |ui| {
                            for (tool, icon, en, tc) in
                                [(QuickTool::Select, "mouse-pointer-2", "Select", "選取"), (QuickTool::Hand, "hand", "Pan", "平移")]
                            {
                                if crate::widgets::icons::button(ui, icon, 34.0, self.quick_tool == tool, label(en, tc)).clicked() {
                                    self.quick_tool = tool;
                                }
                            }
                            ui.add_enabled_ui(!dirty, |ui| {
                                if crate::widgets::icons::button(
                                    ui,
                                    "scan",
                                    34.0,
                                    self.hosted.as_ref().is_some_and(|host| host.selecting),
                                    label("Select plan region", "選擇圖則範圍"),
                                )
                                .clicked()
                                    && let Some(host) = &mut self.hosted
                                {
                                    host.selecting = true;
                                    host.capture = None;
                                    host.drag = None;
                                    self.quick_tool = QuickTool::Select;
                                }
                            });
                        });
                        ui.separator();
                        ui.add_enabled_ui(editable, |ui| {
                            for tool in [
                                CommentTool::Note,
                                CommentTool::Highlight,
                                CommentTool::TextBox,
                                CommentTool::Arrow,
                                CommentTool::Rectangle,
                                CommentTool::Oval,
                                CommentTool::Ink,
                            ] {
                                if crate::widgets::icons::button(
                                    ui,
                                    tool.icon(),
                                    34.0,
                                    self.quick_tool == QuickTool::Comment(tool),
                                    crate::i18n::tr(crate::i18n::Lang::from_pref(&self.language), tool.label()),
                                )
                                .clicked()
                                {
                                    self.quick_tool = QuickTool::Comment(tool);
                                }
                            }
                        });
                    });
                });
            });
        if self.right.is_some() {
            if wide {
                egui::Panel::right("geomon_pdf_review").default_size(260.0).resizable(true).show(ui, |ui| {
                    ui.add_enabled_ui(!pending, |ui| crate::panels::right_panel(self, ui));
                });
            } else {
                let mut open = true;
                let title = if self.right == Some(RightPanel::Layers) { label("Layers", "圖層") } else { label("Comments", "註解") };
                egui::Window::new(title).open(&mut open).collapsible(false).default_width(280.0).default_height(320.0).show(ui.ctx(), |ui| {
                    ui.add_enabled_ui(!pending, |ui| crate::panels::right_panel(self, ui));
                });
                if !open {
                    self.right = None;
                }
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(if colors.dark() { egui::Color32::from_rgb(24, 28, 34) } else { egui::Color32::from_rgb(241, 244, 247) })
                    .inner_margin(0),
            )
            .show(ui, |ui| {
                ui.add_enabled_ui(!pending, |ui| {
                    if let Some(i) = self.active {
                        crate::canvas::document_area(self, i, ui);
                        self.hosted_overlay(ui);
                    } else {
                        ui.centered_and_justified(|ui| ui.label(label("Open a project drawing to begin.", "請開啟專案圖則。")));
                    }
                });
            });
        if !matches!(self.dialog, Some(crate::Dialog::CommentProps)) {
            self.dialog = None;
        }
        crate::dialogs::show(self, ui.ctx());
        if self.hosted.as_ref().is_some_and(|h| h.properties) {
            let mut open = true;
            egui::Window::new(label("Document properties", "文件屬性")).open(&mut open).show(ui.ctx(), |ui| {
                if let Some((_, id)) = self.active_ids()
                    && let Some(doc) = self.session.get(id)
                {
                    ui.label(&doc.name);
                    ui.label(format!("{}: {}", label("Pages", "頁數"), doc.info.pages.len()));
                    if let Some(p) = self.views.first().and_then(|v| doc.info.pages.get(v.current)) {
                        ui.label(format!("{}: {:.2} × {:.2} pt", label("Displayed size", "顯示尺寸"), p.width, p.height));
                        ui.label(format!("CropBox: {:?}", p.crop));
                        ui.label(format!("Rotate: {}°", p.rotation));
                    }
                    ui.label("Based on PDFCraft · MIT / Apache-2.0");
                }
            });
            if let Some(host) = &mut self.hosted {
                host.properties = open;
            }
        }
        crate::widgets::toast(self, ui.ctx());
        self.finish_render_frame(ui.ctx());
    }
    fn hosted_overlay(&mut self, ui: &mut egui::Ui) {
        let Some((index, id)) = self.active_ids() else {
            return;
        };
        let Some(doc) = self.session.get(id) else {
            return;
        };
        let Some(host) = &mut self.hosted else { return };
        for (page, rect) in self.views[index].visible_page_rects() {
            let Some(xf) = self.views[index].page_xform(page) else {
                continue;
            };
            let Some(info) = doc.info.pages.get(page) else {
                continue;
            };
            if let Some((p, region)) = host.selection
                && p == page
            {
                let r = xf.user_rect(&doc.info, page, region.map(|n| n as f32));
                ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 180, 180)), egui::StrokeKind::Middle);
            }
            if !host.capturing() {
                continue;
            }
            let r = rect.intersect(self.views[index].viewport_rect());
            let response = ui.interact(r, ui.scope_id().with(("geomon_capture", page)), egui::Sense::click_and_drag());
            let convert = |p: egui::Pos2| page_point(info, xf.rect, xf.rot, p);
            if response.clicked()
                && let Some(at) = response.interact_pointer_pos()
                && let Some(request) = host.capture.take()
            {
                host.emit("point", json!({"requestId":request,"pageIndex":page,"point":convert(at)}), None);
            }
            if host.selecting
                && response.drag_started()
                && let Some(at) = ui.input(|i| i.pointer.press_origin())
            {
                host.drag = Some((page, convert(at)));
            }
            if let Some((p, start)) = host.drag
                && p == page
                && let Some(at) = response.interact_pointer_pos()
            {
                let end = convert(at);
                let region = [start[0].min(end[0]), start[1].min(end[1]), start[0].max(end[0]), start[1].max(end[1])];
                ui.painter().rect_stroke(
                    xf.user_rect(&doc.info, page, region.map(|n| n as f32)),
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::CYAN),
                    egui::StrokeKind::Middle,
                );
                if response.drag_stopped() {
                    host.drag = None;
                    if valid_region(region) {
                        host.selection = Some((page, region));
                        host.selecting = false;
                        host.emit("selection", json!({"pageIndex":page,"region":region}), None);
                    }
                }
            }
        }
    }
}
