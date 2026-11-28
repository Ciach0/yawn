//! Calls that take their arguments from the answers of earlier calls, in one request. A server
//! function with the default encoding also has a `name_p` twin that returns a [`Pending`] at once;
//! a piece of it ([`Pending::pick`]) goes where an argument does, and the server fills it in from
//! the answer it computed a moment before, so the chain costs one round trip instead of one per
//! call:
//!
//! ```ignore
//! let user = get_user_p(id);
//! let posts = posts_of_p(user.pick(|u| u.id));
//! let posts = posts.await?; // one request carried both calls
//! ```
//!
//! A piece can only be used in the tick it was made in (the calls of a tick are one batch). During
//! server rendering a calls runs where it is, and a piece of a result has nothing to be taken from:
//! it fails.

use super::Error;
use std::marker::PhantomData;
use yawn_bits::{Bits, Field, Pipe};

/// A piece of the answer of a call that has not come back yet
pub struct Piped<F> {
    #[cfg(target_arch = "wasm32")]
    pub(super) call: u32,
    #[cfg(target_arch = "wasm32")]
    pub(super) epoch: u32,
    pub(super) path: Vec<u32>,
    marker: PhantomData<fn() -> F>,
}

impl<F> Clone for Piped<F> {
    fn clone(&self) -> Self {
        Piped {
            #[cfg(target_arch = "wasm32")]
            call: self.call,
            #[cfg(target_arch = "wasm32")]
            epoch: self.epoch,
            path: self.path.clone(),
            marker: PhantomData,
        }
    }
}

impl<F: Pipe> Piped<F> {
    /// A field of this piece, by name: `user.pick(|u| u.profile).pick(|p| p.age)`
    pub fn pick<G>(&self, field: impl FnOnce(F::Fields) -> Field<F, G>) -> Piped<G> {
        let mut path = self.path.clone();
        path.extend_from_slice(field(F::fields()).path());
        Piped {
            #[cfg(target_arch = "wasm32")]
            call: self.call,
            #[cfg(target_arch = "wasm32")]
            epoch: self.epoch,
            path,
            marker: PhantomData,
        }
    }
}

/// What can stand for an argument of type `T` of a `name_p` function: a value of `T`, a piece of
/// an earlier answer or a whole one
pub trait IntoArg<T> {
    #[cfg(target_arch = "wasm32")]
    #[doc(hidden)]
    fn part(self, args: &mut ArgBuilder);

    #[cfg(not(target_arch = "wasm32"))]
    #[doc(hidden)]
    fn value(self) -> Result<T, Error>;
}

impl<T: Bits> IntoArg<T> for T {
    #[cfg(target_arch = "wasm32")]
    fn part(self, args: &mut ArgBuilder) {
        yawn_bits::put_value(&self, &mut args.bytes);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn value(self) -> Result<T, Error> {
        Ok(self)
    }
}

impl<T: Bits> IntoArg<T> for Piped<T> {
    #[cfg(target_arch = "wasm32")]
    fn part(self, args: &mut ArgBuilder) {
        args.piece(&self, T::SCHEMA);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn value(self) -> Result<T, Error> {
        Err(no_value())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn no_value() -> Error {
    Error::Status(500, "a piece of an answer is taken in the browser: it has no value while rendering on the server".into())
}

/// The arguments of a call being put together, with the places that earlier answers fill in
#[cfg(target_arch = "wasm32")]
pub struct ArgBuilder {
    pub(super) bytes: Vec<u8>,
    pub(super) holes: Vec<super::batch::Hole>,
    pub(super) error: Option<Error>,
}

#[cfg(target_arch = "wasm32")]
impl ArgBuilder {
    /// The arguments are a tuple `A` of their types
    pub fn new<A: Bits>() -> Self {
        ArgBuilder { bytes: yawn_bits::header::<A>().to_vec(), holes: Vec::new(), error: None }
    }

    fn piece<F>(&mut self, piped: &Piped<F>, schema: u64) {
        if piped.epoch != super::client::epoch() {
            self.error = Some(Error::Body(
                "a piece of an answer can only be used in the tick it was made in".into(),
            ));
            return;
        }
        self.holes.push(super::batch::Hole {
            at: self.bytes.len(),
            from: piped.call,
            path: piped.path.clone(),
            schema: yawn_bits::folded_schema(schema),
        });
    }
}

/// A call that has been queued and has not come back: await it for the answer
pub struct Pending<T> {
    #[cfg(target_arch = "wasm32")]
    pub(super) inner: super::client::Waiting<T>,
    #[cfg(not(target_arch = "wasm32"))]
    inner: std::pin::Pin<Box<dyn Future<Output = Result<T, Error>>>>,
}

impl<T: Pipe> Pending<T> {
    /// A field of the answer, to go where an argument of another call does
    pub fn pick<G>(&self, field: impl FnOnce(T::Fields) -> Field<T, G>) -> Piped<G> {
        Piped {
            #[cfg(target_arch = "wasm32")]
            call: self.inner.position(),
            #[cfg(target_arch = "wasm32")]
            epoch: self.inner.epoch(),
            path: field(T::fields()).path().to_vec(),
            marker: PhantomData,
        }
    }
}

impl<T: Bits> IntoArg<T> for &Pending<T> {
    #[cfg(target_arch = "wasm32")]
    fn part(self, args: &mut ArgBuilder) {
        let whole = Piped::<T> {
            call: self.inner.position(),
            epoch: self.inner.epoch(),
            path: Vec::new(),
            marker: PhantomData,
        };
        args.piece(&whole, T::SCHEMA);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn value(self) -> Result<T, Error> {
        Err(no_value())
    }
}

impl<T: Bits> Future for Pending<T> {
    type Output = Result<T, Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.inner).poll(cx)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn pending<T>(f: impl Future<Output = Result<T, Error>> + 'static) -> Pending<T> {
    Pending { inner: Box::pin(f) }
}
