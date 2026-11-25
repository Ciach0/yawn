/// Runs `f` once after `ms` milliseconds (a "copied" label going back, a message fading)
pub fn after(ms: i32, f: impl FnOnce() + 'static) {
    #[cfg(target_arch = "wasm32")]
    {
        let owner = crate::ui::ambient::capture_owner();
        let timer = wasm_bindgen::closure::Closure::<dyn FnMut()>::once(move || {
            crate::ui::ambient::run_in_owner(owner, || crate::ui::batch(f));
        });
        crate::dom::after_ms(&timer, ms);
        timer.forget();
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (ms, f);
}

/// Waits `ms` milliseconds inside an `async` block (run with
/// [`platform::spawn`](crate::browser::platform::spawn))
pub fn sleep(ms: i32) -> impl std::future::Future<Output = ()> {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::task::{Poll, Waker};

    struct State {
        done: bool,
        waker: Option<Waker>,
    }
    let state = Rc::new(RefCell::new(State { done: false, waker: None }));
    let fire = state.clone();
    after(ms, move || {
        let mut s = fire.borrow_mut();
        s.done = true;
        if let Some(w) = s.waker.take() {
            w.wake();
        }
    });
    std::future::poll_fn(move |cx| {
        let mut s = state.borrow_mut();
        if s.done {
            Poll::Ready(())
        } else {
            s.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    })
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn defer_drop<T: 'static>(value: T) {
    let timer = wasm_bindgen::closure::Closure::<dyn FnMut()>::once(move || drop(value));
    crate::dom::after_ms(&timer, 0);
    timer.forget();
}
