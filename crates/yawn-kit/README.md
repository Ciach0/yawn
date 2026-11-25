# yawn-kit

Browser tools for Yawn applications, written only on `yawn`'s public API: keyboard shortcuts, swipes,
dismiss-on-outside-click, clipboard, pointer and drag-and-drop helpers, element measurement, viewport
size and media-query signals, and (feature `editing`, on by default) a rich text box with selection and
caret helpers.

```toml
[dependencies]
yawn-kit = "0.1"                                  # everything
yawn-kit = { version = "0.1", default-features = false }   # without the text editing helpers
```
