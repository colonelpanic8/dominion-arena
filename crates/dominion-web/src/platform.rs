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

    pub fn session_get(key: &str) -> Option<String> {
        window()?.session_storage().ok()??.get_item(key).ok()?
    }

    pub fn session_set(key: &str, value: &str) {
        if let Some(s) = window().and_then(|w| w.session_storage().ok().flatten()) {
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

    /// Puts `text` on the clipboard: the Clipboard API where the page may use it (a
    /// secure context), else a selected textarea and `execCommand`. False if neither
    /// worked.
    pub fn copy(text: &str) -> bool {
        let f = js_sys::Function::new_with_args("t", COPY_JS);
        f.call1(
            &wasm_bindgen::JsValue::NULL,
            &wasm_bindgen::JsValue::from_str(text),
        )
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    }

    const COPY_JS: &str = r#"
        if (window.isSecureContext && navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(t).catch(() => {});
            return true;
        }
        const a = document.createElement('textarea');
        a.value = t;
        a.setAttribute('readonly', '');
        a.style.cssText = 'position:fixed;top:-1000px;left:0;opacity:0';
        document.body.appendChild(a);
        a.select();
        let ok = false;
        try { ok = document.execCommand('copy'); } catch (e) {}
        a.remove();
        return ok;
    "#;

    /// Covers the page with a notice when the app has crashed (a panic, or a script
    /// error that leaves it unusable), instead of leaving it on screen unresponsive.
    pub fn install_crash_notice() {
        run_js(CRASH_JS);
        std::panic::set_hook(Box::new(|info| {
            let msg = info.to_string();
            if let Some(f) = js_sys::Reflect::get(&js_sys::global(), &"__dominionCrashed".into())
                .ok()
                .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
            {
                let _ = f.call1(&wasm_bindgen::JsValue::NULL, &msg.into());
            }
        }));
    }

    use wasm_bindgen::JsCast;

    const CRASH_JS: &str = r#"
        window.__dominionCrashed = function (msg) {
            if (document.getElementById('crash-notice')) return;
            console.error('dominion-web crashed: ' + msg);
            const d = document.createElement('div');
            d.id = 'crash-notice';
            d.setAttribute('role', 'alert');
            d.innerHTML = '<p>The page stopped working: <code></code></p><button>Reload</button>';
            d.querySelector('code').textContent = msg;
            d.querySelector('button').onclick = () => location.reload();
            document.body.appendChild(d);
        };
        window.addEventListener('error', (e) => {
            const stack = (e.error && e.error.stack) || '';
            if (/unreachable|RuntimeError/.test(e.message || '') || /wasm-function/.test(stack)) {
                window.__dominionCrashed(e.message);
            }
        });
    "#;

    pub fn encode(s: &str) -> String {
        js_sys::encode_uri_component(s).into()
    }

    pub fn scroll_to_top() {
        if let Some(w) = window() {
            w.scroll_to_with_x_and_y(0.0, 0.0);
        }
    }

    /// A seed for previews that need no reproducibility.
    pub fn random_seed() -> u64 {
        let half = || (js_sys::Math::random() * 4_294_967_296.0) as u64;
        (half() << 32) | half()
    }

    /// Runs a script right away, before anything else happens on the page (Dioxus's
    /// `eval` may wait for the next render).
    pub fn run_js(code: &str) {
        let _ = js_sys::eval(code);
    }

    /// Evaluates a script to a boolean (false if it fails).
    pub fn js_bool(code: &str) -> bool {
        js_sys::eval(code)
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
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

    pub fn session_get(key: &str) -> Option<String> {
        storage_get(&format!("session:{key}"))
    }

    pub fn session_set(key: &str, value: &str) {
        storage_set(&format!("session:{key}"), value);
    }

    pub fn capture_fragment_token() {}

    pub fn take_fragment_token() -> Option<String> {
        None
    }

    pub fn origin() -> String {
        String::new()
    }

    pub fn copy(_text: &str) -> bool {
        false
    }

    pub fn install_crash_notice() {}

    pub fn scroll_to_top() {}

    pub fn random_seed() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
    }

    pub fn viewport() -> (f64, f64) {
        (1280.0, 800.0)
    }

    pub fn run_js(_code: &str) {}

    pub fn js_bool(_code: &str) -> bool {
        false
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
