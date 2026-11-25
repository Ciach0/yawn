#[cfg(target_arch = "wasm32")]
mod js {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
export function runScript(code) { try { return new Function(code)(); } catch (e) { console.error('script failed', e); } }
export function scriptValue(code) { try { var v = new Function(code)(); return v === undefined || v === null ? '' : String(v); } catch (e) { console.error('script failed', e); return ''; } }
"#)]
    extern "C" {
        #[wasm_bindgen(js_name = runScript)]
        pub fn run_script(code: &str);
        #[wasm_bindgen(js_name = scriptValue)]
        pub fn script_value(code: &str) -> String;
    }
}

/// Runs `code` (the body of a function) in the page
pub fn run(code: &str) {
    #[cfg(target_arch = "wasm32")]
    js::run_script(code);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = code;
}

/// Runs `code` and gives back what it `return`s as text; empty when it returns nothing or fails,
/// and on the server
pub fn value(code: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    return js::script_value(code);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = code;
        String::new()
    }
}
