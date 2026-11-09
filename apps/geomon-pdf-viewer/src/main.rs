#[cfg(target_arch = "wasm32")]
mod web {
    use eframe::wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use pdfcraft_ui_egui::{
        PdfCraftApp,
        hosted::{Hosted, Inbox, Input, PROTOCOL_VERSION},
    };
    use serde_json::json;
    use std::sync::Arc;

    #[derive(Clone)]
    struct Bridge {
        window: web_sys::Window,
        parent: web_sys::Window,
        origin: String,
    }
    impl Bridge {
        fn new() -> Option<Self> {
            let window = web_sys::window()?;
            let parent = window.parent().ok().flatten()?;
            let origin = window.location().origin().ok()?;
            if js_sys::Object::is(&window, &parent) || parent.location().origin().ok()? != origin {
                return None;
            }
            Some(Self { window, parent, origin })
        }
        fn post(&self, message: serde_json::Value, bytes: Option<Arc<Vec<u8>>>) {
            let Ok(object) = js_sys::JSON::parse(&message.to_string()) else {
                return;
            };
            if let Some(bytes) = bytes {
                let array = js_sys::Uint8Array::from(bytes.as_slice());
                let buffer = array.buffer();
                if js_sys::Reflect::set(&object, &"bytes".into(), &buffer).is_err() {
                    return;
                }
                let _ = self.parent.post_message_with_transfer(&object, &self.origin, &js_sys::Array::of1(&buffer));
            } else {
                let _ = self.parent.post_message(&object, &self.origin);
            }
        }
        fn listen(&self, inbox: Inbox, ctx: egui::Context) {
            let parent = self.parent.clone();
            let origin = self.origin.clone();
            let callback = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                    return;
                }
                let data = event.data();
                if js_sys::Reflect::get(&data, &"protocolVersion".into()).ok().and_then(|v| v.as_f64()) != Some(PROTOCOL_VERSION as f64) {
                    return;
                }
                let Ok(text) = js_sys::JSON::stringify(&data) else {
                    return;
                };
                let Some(text) = text.as_string() else { return };
                let Ok(mut message) = serde_json::from_str::<Input>(&text) else {
                    return;
                };
                if let Input::Open { bytes, .. } = &mut message {
                    let Ok(value) = js_sys::Reflect::get(&data, &"bytes".into()) else {
                        return;
                    };
                    if !value.is_instance_of::<js_sys::ArrayBuffer>() {
                        return;
                    }
                    let array = js_sys::Uint8Array::new(&value);
                    if array.length() == 0 || array.length() > 100 * 1024 * 1024 {
                        return;
                    }
                    *bytes = array.to_vec();
                }
                if let Ok(mut queue) = inbox.lock() {
                    queue.push(message);
                }
                ctx.request_repaint();
            });
            if self.window.add_event_listener_with_callback("message", callback.as_ref().unchecked_ref()).is_ok() {
                callback.forget();
            }
        }
    }
    pub fn start() {
        let Some(bridge) = Bridge::new() else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let Some(canvas) = bridge
                .window
                .document()
                .and_then(|d| d.get_element_by_id("geomon-pdf"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            else {
                return;
            };
            let options = eframe::WebOptions { renderer: eframe::Renderer::Glow, ..Default::default() };
            let failed_bridge = bridge.clone();
            let result = eframe::WebRunner::new()
                .start(
                    canvas,
                    options,
                    Box::new(move |cc| {
                        let mut app = PdfCraftApp::new();
                        app.session.set_javascript(false);
                        let inbox = Inbox::default();
                        bridge.listen(inbox.clone(), cc.egui_ctx.clone());
                        let output = bridge.clone();
                        app.hosted = Some(Hosted::new(inbox, Box::new(move |value, bytes| output.post(value, bytes))));
                        let unsaved = app.unsaved_flag.clone();
                        let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                            if unsaved.load(std::sync::atomic::Ordering::Relaxed) {
                                event.prevent_default();
                            }
                        });
                        if bridge.window.add_event_listener_with_callback("beforeunload", callback.as_ref().unchecked_ref()).is_ok() {
                            callback.forget();
                        }
                        bridge.post(json!({"type":"geomon-pdf:ready","protocolVersion":PROTOCOL_VERSION}), None);
                        Ok(Box::new(app))
                    }),
                )
                .await;
            match result {
                Ok(()) => {
                    if let Some(status) = failed_bridge.window.document().and_then(|d| d.get_element_by_id("status")) {
                        status.remove();
                    }
                }
                Err(error) => {
                    let message = "The PDF viewer could not start. Check WebGL support and reload the drawing.";
                    if let Some(status) = failed_bridge.window.document().and_then(|d| d.get_element_by_id("status")) {
                        status.set_text_content(Some(message));
                    }
                    failed_bridge.post(json!({"type":"geomon-pdf:startup-failed","protocolVersion":PROTOCOL_VERSION,"error":message}), None);
                    web_sys::console::error_1(&JsValue::from_str(&format!("GeoMon PDF Viewer: {error:?}")));
                }
            }
        });
    }
}
#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}
#[cfg(not(target_arch = "wasm32"))]
fn main() {}
