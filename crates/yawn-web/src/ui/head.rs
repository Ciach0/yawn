use super::rt::with_backend;

/// `<meta name="description" content="...">`
pub fn meta(name: &str, content: &str) {
    with_backend(|b| b.head_tag("meta", &[("name", name), ("content", content)]));
}

/// `<meta property="og:title" content="...">` (Open Graph)
pub fn property(property: &str, content: &str) {
    with_backend(|b| b.head_tag("meta", &[("property", property), ("content", content)]));
}

/// `<link rel="..." href="...">`, once for each distinct pair
pub fn link(rel: &str, href: &str) {
    let href = crate::assets::for_attr(href);
    with_backend(|b| b.head_tag("link", &[("rel", rel), ("href", &href)]));
}

/// `<link rel="stylesheet" href="...">`, added once however many components ask for it: a component
/// that needs a stylesheet or web fonts only on the pages where it is drawn requests them here
pub fn stylesheet(href: &str) {
    link("stylesheet", href);
}

/// Asks the browser to fetch a font file now, before the stylesheet that uses it has been read and
/// a glyph needs it
pub fn preload_font(href: &str) {
    let url = crate::assets::for_attr(href);
    with_backend(|b| {
        b.head_tag(
            "link",
            &[
                ("rel", "preload"),
                ("as", "font"),
                ("type", "font/woff2"),
                ("crossorigin", ""),
                ("href", &url),
            ],
        )
    });
}

/// Asks the browser to download `url` before the client starts and keeps the text under `name`; the
/// client reads it with `yawn::web::boot(name)`
pub fn boot(name: &str, url: &str) {
    let url = crate::assets::for_attr(url);
    with_backend(|b| {
        b.head_tag(
            "link",
            &[
                ("rel", "preload"),
                ("as", "fetch"),
                ("crossorigin", ""),
                ("href", &url),
                ("data-yawn-boot", name),
            ],
        )
    });
}
