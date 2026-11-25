pub use yawn::ui::Rect;

#[cfg(target_arch = "wasm32")]
use crate::glue::page as glue;

/// The value of `attr` on the element under the viewport point `(x, y)` or the nearest ancestor
/// that has it
pub fn attribute_under(x: f64, y: f64, attr: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    return glue::under_pointer(x, y, attr);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (x, y, attr);
        None
    }
}

/// Where the caret of the editable box with this `id` is, measured now
#[cfg(feature = "editing")]
pub fn caret_rect(id: &str) -> Option<Rect> {
    #[cfg(target_arch = "wasm32")]
    {
        let [left, top, right, bottom] = crate::glue::editing::caret_rect(id)?;
        Some(Rect { left, top, right, bottom })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
        None
    }
}

/// Keeps the on-screen keyboard from covering the field being typed in: room is made below the
/// content and the field is scrolled into the part of the window that stays visible
pub fn keep_focus_visible() {
    #[cfg(target_arch = "wasm32")]
    glue::keep_focus_visible();
}

/// Where a floating panel goes relative to the element it opens from
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    /// Open upward: below the element there's too little room and above it there's more
    pub above: bool,
    /// How far, in pixels, to slide the panel sideways (positive is right) to keep it inside the
    /// window
    pub shift: f64,
}

/// Which edge of the element a panel lines up with
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Its left edge with the element's left edge
    Start,
    /// Its middle with the element's middle
    Center,
    /// Its right edge with the element's right edge
    End,
}

/// Where a panel `width` by `height` pixels fits when it opens from the element with this `id`: on
/// the side with room, and slid sideways so it stays 8px inside the window's edges
pub fn place(id: &str, width: f64, height: f64, align: Align) -> Placement {
    place_with(id, width, height, align, 8.0)
}

/// [`place`] with the gap to keep from the window's edges (pixels) given, for a panel that wants
/// more air than 8px
pub fn place_with(id: &str, width: f64, height: f64, align: Align, margin: f64) -> Placement {
    match yawn::ui::element(id).and_then(|e| e.rect()) {
        Some(r) => place_rect_with(r, width, height, align, margin),
        None => Placement::default(),
    }
}

/// [`place`] for an element that's already measured (for instance with
/// [`NodeRef::rect`](yawn::ui::NodeRef::rect) or
/// [`Event::target_rect`](yawn::ui::Event::target_rect)), so the element needs no `id`
pub fn place_rect(r: Rect, width: f64, height: f64, align: Align) -> Placement {
    place_rect_with(r, width, height, align, 8.0)
}

/// [`place_rect`] with the gap to keep from the window's edges given
pub fn place_rect_with(r: Rect, width: f64, height: f64, align: Align, margin: f64) -> Placement {
    let edge = margin;
    let size = crate::viewport::size();
    let (vw, vh) = (size.width, size.height);
    let below = vh - r.bottom - edge;
    let above_room = r.top - edge;
    let above = below < height && above_room > below;
    let left = match align {
        Align::Start => r.left,
        Align::Center => (r.left + r.right) / 2.0 - width / 2.0,
        Align::End => r.right - width,
    };
    let shift = if width > vw - 2.0 * edge || left < edge {
        edge - left
    } else if left + width > vw - edge {
        vw - edge - (left + width)
    } else {
        0.0
    };
    Placement { above, shift }
}
