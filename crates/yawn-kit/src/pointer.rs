use std::cell::{Cell, RefCell};
use std::rc::Rc;
use yawn::browser::global::{Target, listen_global, swallow_next_click};

/// Where a drag is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// The pointer went down on the element
    Start,
    /// The pointer moved while down, anywhere on the page
    Move,
    /// The pointer went up, or the browser took the gesture away
    End,
}

/// One step of a drag
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub phase: DragPhase,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Calls `handle` while the user drags on the element `element_id`. The element is looked up on every event, so it may be created
/// later than this call (a panel that's built on first open). Give it `touch-action: none` (Tailwind: `touch-none`), or a finger
/// scrolls the page. Listens until the owner (the component) is disposed. Does nothing on the server.
///
/// ```ignore
/// on_drag("hue-ring", move |d| hue.set(d.x / d.width));
/// ```
pub fn on_drag(element_id: &str, handle: impl FnMut(Drag) + 'static) {
    let element_id: Rc<str> = element_id.into();
    let handle = Rc::new(RefCell::new(handle));
    let down = Rc::new(Cell::new(false));
    // reports a step, with the pointer's place measured from the element's top left
    let fire = {
        let (handle, element_id) = (handle.clone(), element_id.clone());
        move |ev: &yawn::browser::global::RawEvent, phase: DragPhase| {
            let Some(r) = yawn::ui::element(&element_id).and_then(|e| e.rect()) else {
                return;
            };
            let drag = Drag {
                phase,
                x: ev.number(&["clientX"]).unwrap_or(0.0) - r.left,
                y: ev.number(&["clientY"]).unwrap_or(0.0) - r.top,
                width: r.width(),
                height: r.height(),
            };
            (handle.borrow_mut())(drag);
        }
    };
    {
        let (down, fire, element_id) = (down.clone(), fire.clone(), element_id.clone());
        listen_global(Target::Document, "pointerdown", false, move |ev| {
            // the main button only
            if ev.inside(&element_id) && ev.number(&["button"]) == Some(0.0) {
                down.set(true);
                ev.prevent_default();
                fire(ev, DragPhase::Start);
            }
        });
    }
    {
        let (down, fire) = (down.clone(), fire.clone());
        listen_global(Target::Document, "pointermove", false, move |ev| {
            if down.get() {
                ev.prevent_default();
                fire(ev, DragPhase::Move);
            }
        });
    }
    for kind in ["pointerup", "pointercancel"] {
        let (down, fire) = (down.clone(), fire.clone());
        listen_global(Target::Document, kind, false, move |ev| {
            if down.replace(false) {
                fire(ev, DragPhase::End);
                swallow_next_click();
            }
        });
    }
}
