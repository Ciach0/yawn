use crate::measure::attribute_under;
use std::cell::RefCell;
use std::rc::Rc;
use yawn::browser::global::{Target, listen_global, swallow_next_click};

/// Where a drag is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragStep {
    /// The pointer moved far enough from where it went down: the drag begins
    Start,
    Move,
    /// The pointer went up over a drop zone (or anywhere, `over` says)
    Drop,
    /// The browser took the gesture away, or Escape was pressed
    Cancel,
}

/// One step of a drag
#[derive(Clone, Debug, PartialEq)]
pub struct DragDrop {
    pub step: DragStep,
    /// `data-drag` of what is carried
    pub source: String,
    /// `data-drop` of the zone under the pointer, if any
    pub over: Option<String>,
    /// Pointer position in the viewport
    pub x: f64,
    pub y: f64,
}

/// Pixels the pointer must travel before a press becomes a drag
pub const THRESHOLD: f64 = 5.0;

/// Calls `handle` for every step of a drag, until the owner (the component) is disposed
pub fn on_drag_drop(handle: impl FnMut(DragDrop) + 'static) {
    let handle = Rc::new(RefCell::new(handle));
    let press: Rc<RefCell<Option<Press>>> = Rc::new(RefCell::new(None));
    let fire = {
        let handle = handle.clone();
        move |step: DragStep, source: &str, x: f64, y: f64| {
            let over =
                if step == DragStep::Cancel { None } else { attribute_under(x, y, "data-drop") };
            (handle.borrow_mut())(DragDrop { step, source: source.to_string(), over, x, y });
        }
    };
    {
        let press = press.clone();
        listen_global(Target::Document, "pointerdown", false, move |ev| {
            if ev.number(&["button"]) != Some(0.0) {
                return;
            }
            let (x, y) =
                (ev.number(&["clientX"]).unwrap_or(0.0), ev.number(&["clientY"]).unwrap_or(0.0));
            if let Some(source) = attribute_under(x, y, "data-drag") {
                *press.borrow_mut() = Some(Press { source, x, y, started: false });
            }
        });
    }
    {
        let (press, fire) = (press.clone(), fire.clone());
        listen_global(Target::Document, "pointermove", false, move |ev| {
            let (x, y) =
                (ev.number(&["clientX"]).unwrap_or(0.0), ev.number(&["clientY"]).unwrap_or(0.0));
            let mut slot = press.borrow_mut();
            let Some(p) = slot.as_mut() else { return };
            let first = !p.started;
            // a press becomes a drag only once the pointer has moved a few pixels, so a plain click still clicks
            if first {
                if (x - p.x).hypot(y - p.y) < THRESHOLD {
                    return;
                }
                p.started = true;
            }
            let source = p.source.clone();
            drop(slot);
            ev.prevent_default();
            fire(if first { DragStep::Start } else { DragStep::Move }, &source, x, y);
        });
    }
    let finish = {
        let (press, fire) = (press.clone(), fire.clone());
        move |step: DragStep, x: f64, y: f64| {
            let Some(p) = press.borrow_mut().take() else {
                return;
            };
            if p.started {
                fire(step, &p.source, x, y);
                swallow_next_click();
            }
        }
    };
    {
        let finish = finish.clone();
        listen_global(Target::Document, "pointerup", false, move |ev| {
            finish(
                DragStep::Drop,
                ev.number(&["clientX"]).unwrap_or(0.0),
                ev.number(&["clientY"]).unwrap_or(0.0),
            );
        });
    }
    {
        let finish = finish.clone();
        listen_global(Target::Document, "pointercancel", false, move |ev| {
            finish(
                DragStep::Cancel,
                ev.number(&["clientX"]).unwrap_or(0.0),
                ev.number(&["clientY"]).unwrap_or(0.0),
            );
        });
    }
    listen_global(Target::Document, "keydown", false, move |ev| {
        if ev.string(&["key"]).as_deref() == Some("Escape") {
            finish(DragStep::Cancel, 0.0, 0.0);
        }
    });
}

struct Press {
    source: String,
    x: f64,
    y: f64,
    started: bool,
}
