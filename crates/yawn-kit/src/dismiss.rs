use yawn::browser::global::{Target, listen_global};

/// Calls `close` when the user clicks outside the element `container_id`, or presses Escape.
/// Listens until the owner (the component) is disposed. Does nothing on the server.
///
/// Call it from a component's setup and keep the trigger inside the container, so that a click on
/// the trigger isn't an outside click:
///
/// ```ignore
/// let open = signal(false);
/// on_dismiss("account-menu", move || open.set(false));
/// ```
pub fn on_dismiss(container_id: &'static str, close: impl FnMut() + 'static) {
    let close = std::rc::Rc::new(std::cell::RefCell::new(close));
    {
        let close = close.clone();
        listen_global(Target::Document, "click", false, move |ev| {
            if !ev.inside_or_gone(container_id) {
                (close.borrow_mut())();
            }
        });
    }
    listen_global(Target::Document, "keydown", false, move |ev| {
        if ev.string(&["key"]).as_deref() == Some("Escape") {
            (close.borrow_mut())();
        }
    });
}
