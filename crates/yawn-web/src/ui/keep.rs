use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;

const ENTRY: char = '\u{1e}';
const FIELD: char = '\u{1f}';
const ITEM: char = '\u{2}';
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const WINDOW_KEY: &str = "__yawnKept";
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const LIVE_MS: i32 = 8000;
const CARRY_MS: i32 = 6000;

#[derive(Default)]
struct Keep {
    enabled: bool,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    started: bool,
    path: String,
    saved: HashMap<String, String>,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    all: HashMap<String, String>,
    counts: HashMap<String, u32>,
    carry_id: u32,
    initial: HashMap<String, String>,
    saved_initial: HashMap<String, String>,
    live: Vec<(String, Box<dyn Fn() -> Option<String>>)>,
}

thread_local! {
    static KEEP: RefCell<Keep> = RefCell::new(Keep::default());
}

fn clean(text: &str) -> bool {
    !text.contains(ENTRY) && !text.contains(FIELD) && !text.contains(ITEM)
}

pub(crate) fn encode<T: 'static>(value: &T) -> Option<String> {
    let any = value as &dyn Any;
    macro_rules! number {
        ($($t:ty => $tag:literal),*) => {$(
            if let Some(v) = any.downcast_ref::<$t>() { return Some(format!("{}{}", $tag, v)); }
        )*};
    }
    number!(bool => 'b', i32 => 'i', i64 => 'l', u32 => 'u', u64 => 'w', usize => 'z', f64 => 'f');
    if let Some(v) = any.downcast_ref::<String>() {
        return clean(v).then(|| format!("s{v}"));
    }
    if let Some(v) = any.downcast_ref::<Option<String>>() {
        return match v {
            None => Some("n".into()),
            Some(v) => clean(v).then(|| format!("S{v}")),
        };
    }
    macro_rules! optional {
        ($($t:ty),*) => {$(
            if let Some(v) = any.downcast_ref::<Option<$t>>() {
                return Some(match v { None => "n".to_string(), Some(v) => format!("O{v}") });
            }
        )*};
    }
    optional!(usize, i64, u64, u32, i32, bool, f64);
    if let Some(v) = any.downcast_ref::<Vec<String>>() {
        return v.iter().all(|s| clean(s)).then(|| format!("v{}", v.join(&ITEM.to_string())));
    }
    None
}

pub(crate) fn decode<T: 'static>(text: &str) -> Option<T> {
    let mut chars = text.chars();
    let tag = chars.next()?;
    let rest = chars.as_str();
    let candidates: Vec<Box<dyn Any>> = match tag {
        'b' => vec![Box::new(rest == "true")],
        'i' => vec![Box::new(rest.parse::<i32>().ok()?)],
        'l' => vec![Box::new(rest.parse::<i64>().ok()?)],
        'u' => vec![Box::new(rest.parse::<u32>().ok()?)],
        'w' => vec![Box::new(rest.parse::<u64>().ok()?)],
        'z' => vec![Box::new(rest.parse::<usize>().ok()?)],
        'f' => vec![Box::new(rest.parse::<f64>().ok()?)],
        's' => vec![Box::new(rest.to_string())],
        'S' => vec![Box::new(Some(rest.to_string()))],
        'n' => vec![
            Box::new(None::<String>),
            Box::new(None::<usize>),
            Box::new(None::<i64>),
            Box::new(None::<u64>),
            Box::new(None::<u32>),
            Box::new(None::<i32>),
            Box::new(None::<bool>),
            Box::new(None::<f64>),
        ],
        'O' => {
            let mut out: Vec<Box<dyn Any>> = Vec::new();
            if let Ok(v) = rest.parse::<usize>() {
                out.push(Box::new(Some(v)));
            }
            if let Ok(v) = rest.parse::<i64>() {
                out.push(Box::new(Some(v)));
            }
            if let Ok(v) = rest.parse::<u64>() {
                out.push(Box::new(Some(v)));
            }
            if let Ok(v) = rest.parse::<u32>() {
                out.push(Box::new(Some(v)));
            }
            if let Ok(v) = rest.parse::<i32>() {
                out.push(Box::new(Some(v)));
            }
            if let Ok(v) = rest.parse::<bool>() {
                out.push(Box::new(Some(v)));
            }
            if let Ok(v) = rest.parse::<f64>() {
                out.push(Box::new(Some(v)));
            }
            out
        }
        'v' => vec![Box::new(if rest.is_empty() {
            Vec::<String>::new()
        } else {
            rest.split(ITEM).map(str::to_string).collect()
        })],
        _ => return None,
    };
    candidates.into_iter().find_map(|c| c.downcast::<T>().ok().map(|b| *b))
}

pub(crate) fn supported<T: 'static>() -> bool {
    use std::any::TypeId;
    let id = TypeId::of::<T>();
    [
        TypeId::of::<bool>(),
        TypeId::of::<i32>(),
        TypeId::of::<i64>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<usize>(),
        TypeId::of::<f64>(),
        TypeId::of::<String>(),
        TypeId::of::<Option<String>>(),
        TypeId::of::<Option<usize>>(),
        TypeId::of::<Option<i64>>(),
        TypeId::of::<Option<u64>>(),
        TypeId::of::<Option<u32>>(),
        TypeId::of::<Option<i32>>(),
        TypeId::of::<Option<bool>>(),
        TypeId::of::<Option<f64>>(),
        TypeId::of::<Vec<String>>(),
    ]
    .contains(&id)
}

pub(crate) fn active() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::has_dev_literals() || KEEP.with(|k| k.borrow().enabled)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

#[cfg(target_arch = "wasm32")]
fn path() -> String {
    use crate::dom;
    dom::window()
        .and_then(|w| dom::get_object(&w, "location"))
        .and_then(|l| dom::get_string(&l, "pathname"))
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
fn start(keep: &mut Keep) {
    use crate::dom;
    keep.started = true;
    keep.path = path();
    let Some(win) = dom::window() else { return };
    let Some(blob) = dom::get_string(&win, WINDOW_KEY) else {
        return;
    };
    dom::set_string(&win, WINDOW_KEY, "");
    for entry in blob.split(ENTRY) {
        if let Some((key, value)) = entry.split_once(FIELD) {
            keep.saved.insert(key.to_string(), value.to_string());
        }
    }
    keep.all = keep.saved.clone();
    dom::warn(&format!("yawn: restoring {} signals", keep.saved.len()));
    crate::ui::timer::after(LIVE_MS, || KEEP.with(|k| k.borrow_mut().saved.clear()));
}

pub(crate) fn claim<T: 'static>(
    place: &std::panic::Location<'_>,
    fresh: Option<&str>,
) -> (String, Option<T>) {
    KEEP.with(|k| {
        let mut keep = k.borrow_mut();
        #[cfg(target_arch = "wasm32")]
        if !keep.started {
            start(&mut keep);
        }
        let base = format!("{}|{}:{}:{}", keep.path, place.file(), place.line(), place.column());
        let n = keep.counts.entry(base.clone()).or_insert(0);
        let key = format!("{base}#{n}");
        *n += 1;
        let was = keep.saved_initial.remove(&key);
        let text = keep.saved.remove(&key).filter(
            |_| !matches!((was.as_deref(), fresh), (Some(was), Some(fresh)) if was != fresh),
        );
        let value = text.and_then(|text| decode::<T>(&text));
        (key, value)
    })
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn restart() {
    KEEP.with(|k| {
        let mut keep = k.borrow_mut();
        keep.counts.clear();
        keep.live.clear();
        keep.saved = keep.all.clone();
    });
}

pub(crate) fn register(
    key: String,
    read: Box<dyn Fn() -> Option<String>>,
    initial: Option<String>,
) {
    KEEP.with(|k| {
        let mut keep = k.borrow_mut();
        if let Some(initial) = initial {
            keep.initial.insert(key.clone(), initial);
        }
        keep.live.push((key, read));
        if keep.live.len() % 2048 == 0 {
            keep.live.retain(|(_, read)| crate::ui::ambient::run(read).is_some());
        }
    });
}

/// From now on the signals made by the application can be carried over a rebuild of the page
pub fn enable_carry() {
    KEEP.with(|k| k.borrow_mut().enabled = true);
}

/// Runs `rebuild`, which drops the views and builds them again (a change of language does that),
/// and starts every signal of the new build at the value its counterpart in the old build had, so
/// the page comes back as the person left it
pub fn carry<R>(rebuild: impl FnOnce() -> R) -> R {
    stash();
    let out = rebuild();
    // a route may build its page after `rebuild` returns, so saved values stay available for a while
    let id = KEEP.with(|k| {
        let mut keep = k.borrow_mut();
        keep.carry_id = keep.carry_id.wrapping_add(1);
        keep.carry_id
    });
    crate::ui::timer::after(CARRY_MS, move || {
        KEEP.with(|k| {
            let mut keep = k.borrow_mut();
            if keep.carry_id == id {
                keep.saved.clear();
                keep.saved_initial.clear();
            }
        });
    });
    out
}

fn stash() {
    let entries: Vec<(String, String)> = KEEP.with(|k| {
        k.borrow()
            .live
            .iter()
            .filter_map(|(key, read)| crate::ui::ambient::run(read).map(|text| (key.clone(), text)))
            .collect()
    });
    KEEP.with(|k| {
        let mut keep = k.borrow_mut();
        let mut next: HashMap<String, u32> = HashMap::new();
        keep.saved.clear();
        keep.saved_initial.clear();
        for (key, text) in entries {
            let initial = keep.initial.get(&key).cloned();
            let base = key.rsplit_once('#').map(|(base, _)| base.to_string()).unwrap_or(key);
            let n = next.entry(base.clone()).or_insert(0);
            let fresh = format!("{base}#{n}");
            if let Some(initial) = initial {
                keep.saved_initial.insert(fresh.clone(), initial);
            }
            keep.saved.insert(fresh, text);
            *n += 1;
        }
        keep.initial.clear();
        keep.counts.clear();
        keep.live.clear();
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn save() {
    use crate::dom;
    if !active() {
        return;
    }
    let blob = KEEP.with(|k| {
        let keep = k.borrow();
        keep.live
            .iter()
            .filter_map(|(key, read)| {
                crate::ui::ambient::run(read).map(|text| format!("{key}{FIELD}{text}"))
            })
            .collect::<Vec<_>>()
            .join(&ENTRY.to_string())
    });
    // a client that's already gone has nothing left to read: what its disposal saved stays
    if blob.is_empty() {
        return;
    }
    if let Some(win) = dom::window() {
        dom::set_string(&win, WINDOW_KEY, &blob);
        dom::warn(&format!("yawn: kept {} signals", blob.split(ENTRY).count()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        assert_eq!(decode::<bool>(&encode(&true).unwrap()), Some(true));
        assert_eq!(decode::<String>(&encode(&"a b".to_string()).unwrap()), Some("a b".to_string()));
        assert_eq!(decode::<Option<usize>>(&encode(&Some(3usize)).unwrap()), Some(Some(3)));
        assert_eq!(decode::<Option<String>>(&encode(&None::<String>).unwrap()), Some(None));
        assert_eq!(
            decode::<Vec<String>>(&encode(&vec!["a".to_string(), "b".to_string()]).unwrap()),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(decode::<f64>(&encode(&1.5f64).unwrap()), Some(1.5));
    }

    #[test]
    fn a_value_of_another_type_is_not_used() {
        assert_eq!(decode::<i32>(&encode(&"x".to_string()).unwrap()), None);
        assert_eq!(decode::<String>(&encode(&true).unwrap()), None);
    }
}
