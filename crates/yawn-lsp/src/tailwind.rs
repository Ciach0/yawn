use crate::position::Text;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::OnceLock;

const DATA: &str = include_str!("../data/tailwind-classes.txt");
const KIND_VARIANT: u32 = 14;
const KIND_UTILITY: u32 = 12;
const LIMIT: usize = 250;

struct Classes {
    utilities: Vec<&'static str>,
    variants: Vec<&'static str>,
}

fn classes() -> &'static Classes {
    static C: OnceLock<Classes> = OnceLock::new();
    C.get_or_init(|| {
        let (mut utilities, mut variants) = (Vec::new(), Vec::new());
        for line in DATA.lines().filter(|l| !l.is_empty()) {
            match line.strip_prefix(':') {
                Some(v) => variants.push(v),
                None => utilities.push(line),
            }
        }
        Classes { utilities, variants }
    })
}

/// Whether the project that contains `file` turned Tailwind on in its `Yawn.toml`
pub fn enabled_for(file: &Path) -> bool {
    let Some(root) = yawn::config::find_project_root(file.parent().unwrap_or(file)) else {
        return false;
    };
    matches!(yawn::config::ProjectConfig::load(&root), Ok(Some(c)) if c.tailwind.enabled)
}

/// Completion items for the class being typed, which is `src[start..offset]`
pub fn complete(text: &Text, start: usize, offset: usize) -> Vec<Value> {
    let typed = &text.s[start..offset];
    let after = typed.rfind(':').map_or(0, |n| n + 1);
    let word = &typed[after..];
    let from = start + after;
    let range = json!({"start": text.position(from), "end": text.position(offset)});
    let c = classes();
    let mut out = Vec::new();
    let mut push = |label: &str, kind: u32, new_text: String, rank: usize, detail: &str| {
        out.push(json!({
            "label": label,
            "kind": kind,
            "detail": detail,
            "filterText": label,
            "sortText": format!("{rank}{label}"),
            "textEdit": {"range": range, "newText": new_text},
            // a variant is followed by the class it applies to
            "command": if kind == KIND_VARIANT { json!({"title": "suggest", "command": "editor.action.triggerSuggest"}) } else { Value::Null },
        }));
    };
    for v in c.variants.iter().filter(|v| v.starts_with(word)).take(LIMIT / 5) {
        push(&format!("{v}:"), KIND_VARIANT, format!("{v}:"), 1, "variant");
    }
    // `-m-4` style negatives sort after the positive form
    for u in c.utilities.iter().filter(|u| u.starts_with(word)).take(LIMIT) {
        push(u, KIND_UTILITY, (*u).to_string(), if u.starts_with('-') { 3 } else { 2 }, "tailwind");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(src: &str, start: usize) -> Vec<String> {
        let t = Text::new(src.to_string());
        complete(&t, start, src.len())
            .iter()
            .map(|i| i["label"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn list_loads() {
        let c = classes();
        assert!(
            c.utilities.contains(&"flex")
                && c.utilities.contains(&"p-4")
                && c.utilities.contains(&"text-red-500")
        );
        assert!(
            c.variants.contains(&"hover")
                && c.variants.contains(&"md")
                && c.variants.contains(&"dark")
        );
        assert!(c.utilities.len() > 10_000);
    }

    #[test]
    fn utilities_by_prefix() {
        let l = labels("text-red-5", 0);
        assert!(l.contains(&"text-red-500".to_string()), "{l:?}");
        assert!(l.iter().all(|x| x.starts_with("text-red-5")));
    }

    #[test]
    fn variants_offered_and_kept() {
        assert!(labels("ho", 0).contains(&"hover:".to_string()));
        let src = "md:hover:fl";
        let t = Text::new(src.to_string());
        let items = complete(&t, 0, src.len());
        let flex = items.iter().find(|i| i["label"] == "flex").expect("flex");
        // only `fl` is replaced: the edit starts after the last colon
        assert_eq!(flex["textEdit"]["range"]["start"]["character"], 9);
    }

    #[test]
    fn response_is_bounded() {
        assert!(labels("", 0).len() <= LIMIT + LIMIT / 5);
    }
}
