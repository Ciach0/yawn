use super::context;
use super::signals::Sig;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

pub type Pending = Pin<Box<dyn Future<Output = Option<String>>>>;

/// What a route's `preload` hands the router: where some data comes from, without its type
#[derive(Clone)]
pub struct Source {
    name: Rc<str>,
    url: Rc<str>,
    #[cfg(not(target_arch = "wasm32"))]
    server: Rc<dyn Fn() -> Pending>,
}

impl Source {
    #[cfg(feature = "json")]
    fn new<F>(name: String, url: String, server: impl Fn() -> F + 'static) -> Source
    where
        F: Future<Output = Option<String>> + 'static,
    {
        #[cfg(target_arch = "wasm32")]
        let _ = server;
        Source {
            name: Rc::from(name),
            url: Rc::from(url),
            #[cfg(not(target_arch = "wasm32"))]
            server: Rc::new(move || Box::pin(server()) as Pending),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// The text as it came, `None` until it's there: with the page on the server, from the boot
    /// copy or a preload in the browser, else fetched
    pub fn text(&self) -> Sig<Option<String>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ask = self.server.clone();
            context::resource(&self.name, &self.url, move || ask())
        }
        #[cfg(target_arch = "wasm32")]
        context::resource(&self.name, &self.url, || std::future::ready(None::<String>))
    }

    /// Asks for it on the server before the page gets to read it
    pub fn prefetch(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ask = self.server.clone();
            context::prefetch(&self.name, move || ask());
        }
    }

    /// Starts downloading it in the browser, so the page built a moment later finds it
    pub fn preload(&self) {
        #[cfg(target_arch = "wasm32")]
        context::preload_url(&self.name, &self.url);
    }
}

/// One piece of JSON a page is drawn from, parsed to `T`
#[cfg(feature = "json")]
pub struct Loader<T> {
    source: Source,
    _type: std::marker::PhantomData<fn() -> T>,
}

#[cfg(feature = "json")]
impl<T> Clone for Loader<T> {
    fn clone(&self) -> Self {
        Loader { source: self.source.clone(), _type: std::marker::PhantomData }
    }
}

/// A loader named `name` (what the server hands the browser it with and what's cached under),
/// fetched from `url` in the browser and as JSON text by `server` while the page renders on the
/// server
#[cfg(feature = "json")]
pub fn loader<T, F>(
    name: impl Into<String>,
    url: impl Into<String>,
    server: impl Fn() -> F + 'static,
) -> Loader<T>
where
    T: serde::de::DeserializeOwned + Clone + 'static,
    F: Future<Output = Option<String>> + 'static,
{
    Loader { source: Source::new(name.into(), url.into(), server), _type: std::marker::PhantomData }
}

#[cfg(feature = "json")]
fn parse<T: serde::de::DeserializeOwned>(text: Option<String>) -> Option<T> {
    text.and_then(|t| serde_json::from_str::<T>(&t).ok())
}

#[cfg(feature = "json")]
impl<T: serde::de::DeserializeOwned + Clone + 'static> Loader<T> {
    /// The data, `None` until it's there or when it doesn't parse as a `T`
    pub fn read(&self) -> Sig<Option<T>> {
        let text = self.source.text();
        let out = super::signals::signal_always(parse::<T>(text.peek()));
        let first = std::cell::Cell::new(true);
        super::signals::effect(move || {
            let latest = text.get();
            if !first.replace(false) {
                out.set(parse::<T>(latest));
            }
        });
        out
    }
}

#[cfg(feature = "json")]
impl<T> Loader<T> {
    /// The JSON as it came, for a page that takes it apart itself
    pub fn text(&self) -> Sig<Option<String>> {
        self.source.text()
    }

    pub fn source(&self) -> Source {
        self.source.clone()
    }
}

#[cfg(feature = "json")]
impl<T> From<Loader<T>> for Source {
    fn from(loader: Loader<T>) -> Source {
        loader.source
    }
}
