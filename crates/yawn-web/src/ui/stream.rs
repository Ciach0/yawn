/// Opens `url` as an event stream and hands the `data` of every message to `on_message`, inside the
/// runtime
pub fn stream(url: &str, on_message: impl FnMut(String) + 'static) {
    stream_with_state(url, on_message, |_| {});
}

/// Like [`stream`], and `on_state` is called with `true` each time the connection is open (the
/// first time and after every reconnect) and with `false` each time it's lost, so a page that
/// relies on the stream for what it knows can tell when it stopped hearing the server
pub fn stream_with_state(
    url: &str,
    on_message: impl FnMut(String) + 'static,
    on_state: impl FnMut(bool) + 'static,
) {
    #[cfg(target_arch = "wasm32")]
    browser::open(url, on_message, on_state);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (url, on_message, on_state);
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use crate::ui::ambient;
    use crate::ui::signals::{batch, on_cleanup};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
export function streamOpen(url, cb, st) {
  var h = { es: null, closed: false, timer: 0, delay: 1000 };
  function open() {
    if (h.closed) return;
    var es = new EventSource(url);
    h.es = es;
    es.onopen = function () { st(true); };
    es.onmessage = function (e) { h.delay = 1000; cb(e.data); };
    // the browser reconnects by itself after a dropped connection, but gives up for good on an answer that is not a stream (a 502 while
    // the server restarts); that one is tried again here, a little later each time
    es.onerror = function () {
      st(false);
      if (es.readyState === 2) {
        es.close();
        h.timer = setTimeout(open, h.delay);
        h.delay = Math.min(h.delay * 2, 15000);
      }
    };
  }
  open();
  return h;
}
export function streamClose(h) { h.closed = true; clearTimeout(h.timer); if (h.es) h.es.close(); }
"#)]
    extern "C" {
        pub type Source;
        #[wasm_bindgen(js_name = streamOpen)]
        fn stream_open(
            url: &str,
            cb: &Closure<dyn FnMut(String)>,
            st: &Closure<dyn FnMut(bool)>,
        ) -> Source;
        #[wasm_bindgen(js_name = streamClose)]
        fn stream_close(source: &Source);
    }

    pub fn open(
        url: &str,
        mut on_message: impl FnMut(String) + 'static,
        mut on_state: impl FnMut(bool) + 'static,
    ) {
        let owner = ambient::capture_owner();
        let callback = Closure::<dyn FnMut(String)>::new(move |text: String| {
            ambient::run_in_owner(owner, || batch(|| on_message(text)));
        });
        let state = Closure::<dyn FnMut(bool)>::new(move |open: bool| {
            ambient::run_in_owner(owner, || batch(|| on_state(open)));
        });
        let source = stream_open(url, &callback, &state);
        on_cleanup(move || {
            stream_close(&source);
            crate::task::defer_drop(callback);
            crate::task::defer_drop(state);
        });
    }
}
