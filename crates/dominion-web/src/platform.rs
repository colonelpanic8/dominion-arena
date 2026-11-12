//! What the client needs from the browser. Natively (tests, clippy) these are inert
//! stand-ins: storage is an in-memory map and nothing waits.

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod imp {
    use std::cell::RefCell;

    thread_local! {
        static FRAGMENT_TOKEN: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    fn window() -> Option<web_sys::Window> {
        web_sys::window()
    }

    pub async fn sleep(ms: u32) {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }

    pub fn storage_get(key: &str) -> Option<String> {
        window()?.local_storage().ok()??.get_item(key).ok()?
    }

    pub fn storage_set(key: &str, value: &str) {
        if let Some(s) = window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = s.set_item(key, value);
        }
    }

    /// Takes `#token=<token>` out of the address bar, keeping the token for the game page.
    pub fn capture_fragment_token() {
        let Some(w) = window() else { return };
        let location = w.location();
        let Ok(hash) = location.hash() else { return };
        let Some(token) = hash.strip_prefix("#token=") else {
            return;
        };
        let token = js_sys::decode_uri_component(token)
            .ok()
            .and_then(|t| t.as_string())
            .unwrap_or_else(|| token.to_string());
        if token.is_empty() {
            return;
        }
        FRAGMENT_TOKEN.with(|t| *t.borrow_mut() = Some(token));
        let path = location.pathname().unwrap_or_default();
        let search = location.search().unwrap_or_default();
        if let Ok(history) = w.history() {
            let _ = history.replace_state_with_url(
                &wasm_bindgen::JsValue::NULL,
                "",
                Some(&format!("{path}{search}")),
            );
        }
    }

    pub fn take_fragment_token() -> Option<String> {
        FRAGMENT_TOKEN.with(|t| t.borrow_mut().take())
    }

    pub fn origin() -> String {
        window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_default()
    }

    pub fn copy(text: &str) {
        if let Some(w) = window() {
            let _ = w.navigator().clipboard().write_text(text);
        }
    }

    pub fn encode(s: &str) -> String {
        js_sys::encode_uri_component(s).into()
    }

    /// The window's inner size in CSS pixels.
    pub fn viewport() -> (f64, f64) {
        let Some(w) = window() else {
            return (1280.0, 800.0);
        };
        let size = |v: Result<wasm_bindgen::JsValue, _>| v.ok().and_then(|v| v.as_f64());
        (
            size(w.inner_width()).unwrap_or(1280.0),
            size(w.inner_height()).unwrap_or(800.0),
        )
    }
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
mod imp {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    thread_local! {
        static STORAGE: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    }

    pub async fn sleep(_ms: u32) {}

    pub fn storage_get(key: &str) -> Option<String> {
        STORAGE.with(|s| s.borrow().get(key).cloned())
    }

    pub fn storage_set(key: &str, value: &str) {
        STORAGE.with(|s| s.borrow_mut().insert(key.into(), value.into()));
    }

    pub fn capture_fragment_token() {}

    pub fn take_fragment_token() -> Option<String> {
        None
    }

    pub fn origin() -> String {
        String::new()
    }

    pub fn copy(_text: &str) {}

    pub fn viewport() -> (f64, f64) {
        (1280.0, 800.0)
    }

    pub fn encode(s: &str) -> String {
        s.chars()
            .flat_map(|c| {
                if c.is_ascii_alphanumeric() || "-_.~".contains(c) {
                    vec![c]
                } else {
                    let mut buf = [0; 4];
                    c.encode_utf8(&mut buf)
                        .bytes()
                        .flat_map(|b| format!("%{b:02X}").chars().collect::<Vec<_>>())
                        .collect()
                }
            })
            .collect()
    }
}

pub use imp::*;
