//! Replaces the body of a component while the page runs (dev builds only).
//!
//! With `[dev] dynamic` a generated component renders through [`entry`]: when a body was
//! [`register`]ed under the component's type name it runs that one, otherwise the body compiled
//! into the app does. A module loaded later (see `yawn dev`) registers new bodies, so editing a
//! component rebuilds that module and not the app.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn bodies() -> &'static Mutex<HashMap<String, usize>> {
    static BODIES: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    BODIES.get_or_init(Default::default)
}

#[doc(hidden)]
#[inline(never)]
pub fn register_raw(name: &str, body: usize) {
    bodies().lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), body);
}

#[doc(hidden)]
#[inline(never)]
pub fn entry_raw(name: &str) -> Option<usize> {
    bodies().lock().unwrap_or_else(|e| e.into_inner()).get(name).copied()
}

/// Makes `body` the one `T` renders with from now on
pub fn register<T>(name: &str, body: fn(T) -> crate::ui::View) {
    register_raw(name, body as usize);
}

/// The body registered for `T`, if a loaded module replaced it
pub fn entry<T>(name: &str) -> Option<fn(T) -> crate::ui::View> {
    // SAFETY: only `register` stores values, and it stores a `fn(T) -> View` under the name of
    // exactly that `T`.
    entry_raw(name).map(|p| unsafe { std::mem::transmute::<usize, fn(T) -> crate::ui::View>(p) })
}

/// Builds the application again with the bodies registered now, keeping the signals' values. The
/// development client calls it after it loaded new modules.
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
#[unsafe(no_mangle)]
pub extern "C" fn __yawn_hot_refresh() {
    crate::web::remount();
}

/// Memory for the data and stack of a module loaded later (see `yawn dev`)
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
#[unsafe(no_mangle)]
pub extern "C" fn __yawn_alloc(size: usize, align: usize) -> *mut u8 {
    // SAFETY: the loader passes a non-zero size and a power of two alignment
    unsafe { std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(size, align)) }
}

/// What a dev build compiles `thread_local!` to on the server: one value per thread, kept in a
/// named static, so a library loaded later shares it with the app (a real thread local can't be)
#[cfg(not(target_arch = "wasm32"))]
pub struct DevLocal<T: 'static> {
    init: fn() -> T,
    values: Mutex<Vec<(std::thread::ThreadId, &'static T)>>,
}

// SAFETY: a value is only reached from the thread it was made on
#[cfg(not(target_arch = "wasm32"))]
unsafe impl<T> Sync for DevLocal<T> {}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static EXITS: std::cell::RefCell<Exits> = const { std::cell::RefCell::new(Exits(Vec::new())) };
}

#[cfg(not(target_arch = "wasm32"))]
struct Exits(Vec<Box<dyn FnOnce()>>);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Exits {
    fn drop(&mut self) {
        for f in self.0.drain(..) {
            f();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
#[inline(never)]
pub fn on_thread_exit(f: Box<dyn FnOnce()>) {
    let _ = EXITS.try_with(|e| e.borrow_mut().0.push(f));
}

#[cfg(not(target_arch = "wasm32"))]
impl<T: 'static> DevLocal<T> {
    pub const fn new(init: fn() -> T) -> Self {
        Self { init, values: Mutex::new(Vec::new()) }
    }

    pub fn with<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        let id = std::thread::current().id();
        let found = self
            .values
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, v)| *v);
        let value = match found {
            Some(v) => v,
            None => {
                let v: &'static T = Box::leak(Box::new((self.init)()));
                self.values.lock().unwrap_or_else(|e| e.into_inner()).push((id, v));
                let this: &'static Self = self;
                on_thread_exit(Box::new(move || {
                    let mut all = this.values.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(at) = all.iter().position(|(i, _)| *i == id) {
                        let (_, v) = all.swap_remove(at);
                        drop(all);
                        // SAFETY: the value was leaked from a `Box` by `with` and this thread was
                        // its only user
                        drop(unsafe { Box::from_raw(v as *const T as *mut T) });
                    }
                }));
                v
            }
        };
        f(value)
    }

    pub fn try_with<R>(
        &'static self,
        f: impl FnOnce(&T) -> R,
    ) -> Result<R, std::thread::AccessError> {
        Ok(self.with(f))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<T: 'static> DevLocal<std::cell::RefCell<T>> {
    pub fn with_borrow<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        self.with(|c| f(&c.borrow()))
    }
    pub fn with_borrow_mut<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R {
        self.with(|c| f(&mut c.borrow_mut()))
    }
    pub fn set(&'static self, value: T) {
        self.with(|c| *c.borrow_mut() = value)
    }
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(|c| c.take())
    }
    pub fn replace(&'static self, value: T) -> T {
        self.with(|c| c.replace(value))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<T: 'static> DevLocal<std::cell::Cell<T>> {
    pub fn get(&'static self) -> T
    where
        T: Copy,
    {
        self.with(|c| c.get())
    }
    pub fn set(&'static self, value: T) {
        self.with(|c| c.set(value))
    }
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(|c| c.take())
    }
    pub fn replace(&'static self, value: T) -> T {
        self.with(|c| c.replace(value))
    }
}
