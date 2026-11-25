#[cfg(target_arch = "wasm32")]
use crate::ui::Event;
use crate::ui::FileRef;

#[cfg(target_arch = "wasm32")]
mod js {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
export function readFile(file) {
  return new Promise(function (resolve, reject) {
    var r = new FileReader();
    r.onload = function () { resolve(r.result); };
    r.onerror = function () { reject((r.error && r.error.message) || 'cannot read the file'); };
    r.readAsDataURL(file);
  });
}
export function describeEvent(ev) {
  var dt = ev.clipboardData || ev.dataTransfer, text = '';
  try { if (dt) text = dt.getData('text/plain') || ''; } catch (x) {}
  var list = (ev.target && ev.target.files && ev.target.files.length) ? ev.target.files : (dt && dt.files ? dt.files : []);
  return { x: ev.clientX || 0, y: ev.clientY || 0, inputType: ev.inputType || '', text: text, files: Array.prototype.slice.call(list) };
}
export function randomUnit() { return Math.random(); }
export function nowMillis() { return Date.now(); }
export function nowSeconds() { return Math.floor(Date.now() / 1000); }
"#)]
    extern "C" {
        #[wasm_bindgen(js_name = readFile)]
        pub fn read_file(file: &JsValue) -> JsValue;
        #[wasm_bindgen(js_name = describeEvent)]
        pub fn describe_event(ev: &JsValue) -> JsValue;
        #[wasm_bindgen(js_name = randomUnit)]
        pub fn random_unit() -> f64;
        #[wasm_bindgen(js_name = nowMillis)]
        pub fn now_millis() -> f64;
        #[wasm_bindgen(js_name = nowSeconds)]
        pub fn now_seconds() -> f64;
    }
}

/// A random number in `[0, 1)`
pub fn random() -> f64 {
    #[cfg(target_arch = "wasm32")]
    return js::random_unit();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1);
        (n.wrapping_mul(2_654_435_761) % 1_000_000) as f64 / 1_000_000.0
    }
}

/// Runs a future on the page's event loop (a file being read, a request being made)
pub fn spawn(future: impl std::future::Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    crate::dom::spawn_local(future);
    #[cfg(not(target_arch = "wasm32"))]
    drop(future);
}

/// Seconds since 1970-01-01 UTC, by the clock of the page (or of the server)
pub fn now_seconds() -> i64 {
    #[cfg(target_arch = "wasm32")]
    return js::now_seconds() as i64;
    #[cfg(not(target_arch = "wasm32"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Milliseconds since 1970-01-01 UTC, by the clock of the page (or of the server)
pub fn now_millis() -> i64 {
    #[cfg(target_arch = "wasm32")]
    return js::now_millis() as i64;
    #[cfg(not(target_arch = "wasm32"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Reads the whole file as a `data:` URL
pub async fn read_file(file: &FileRef) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let handle =
            file.handle.downcast_ref::<wasm_bindgen::JsValue>().ok_or("not a browser file")?;
        let result = crate::dom::await_promise(js::read_file(handle))
            .await
            .map_err(|e| e.as_string().unwrap_or_else(|| "cannot read the file".to_string()))?;
        result.as_string().ok_or_else(|| "cannot read the file".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = file;
        Err("files can only be read in the browser".into())
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn fill_event(ev: &crate::dom::Event, e: &mut Event) {
    use std::rc::Rc;
    let d = js::describe_event(ev.as_ref());
    e.x = crate::dom::get_number(&d, "x");
    e.y = crate::dom::get_number(&d, "y");
    e.input_type = crate::dom::get_string(&d, "inputType").unwrap_or_default();
    e.text = crate::dom::get_string(&d, "text").unwrap_or_default();
    if let Some(files) = crate::dom::get_object(&d, "files") {
        let len = crate::dom::get_number(&files, "length") as u32;
        for i in 0..len {
            let Some(f) = crate::dom::array_item(&files, i) else {
                continue;
            };
            e.files.push(FileRef {
                name: crate::dom::get_string(&f, "name").unwrap_or_default(),
                size: crate::dom::get_number(&f, "size"),
                mime: crate::dom::get_string(&f, "type").unwrap_or_default(),
                handle: Rc::new(f),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_number_is_a_fraction() {
        assert!((0.0..1.0).contains(&random()));
    }
}
