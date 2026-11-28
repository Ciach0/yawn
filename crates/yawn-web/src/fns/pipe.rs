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
//! In the browser a piece can only be used in the tick it was made in (the calls of a tick are one
//! batch). While rendering on the server a call runs where it is: the piece is cut from the answer
//! of the earlier call (shared by whoever awaits it) once that is there, with the same projection
//! the batch uses.

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
    #[cfg(not(target_arch = "wasm32"))]
    source: std::rc::Rc<dyn Source>,
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
            #[cfg(not(target_arch = "wasm32"))]
            source: self.source.clone(),
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
            #[cfg(not(target_arch = "wasm32"))]
            source: self.source.clone(),
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
    fn value(self) -> ArgFuture<T>;
}

/// The argument of a call while rendering on the server: ready, or waiting for an earlier call
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub type ArgFuture<T> = std::pin::Pin<Box<dyn Future<Output = Result<T, Error>>>>;

impl<T: Bits + 'static> IntoArg<T> for T {
    #[cfg(target_arch = "wasm32")]
    fn part(self, args: &mut ArgBuilder) {
        yawn_bits::put_value(&self, &mut args.bytes);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn value(self) -> ArgFuture<T> {
        Box::pin(std::future::ready(Ok(self)))
    }
}

impl<T: Bits + 'static> IntoArg<T> for Piped<T> {
    #[cfg(target_arch = "wasm32")]
    fn part(self, args: &mut ArgBuilder) {
        args.piece(&self, T::SCHEMA);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn value(self) -> ArgFuture<T> {
        let Piped { source, path, .. } = self;
        Box::pin(async move { read::<T>(&*source, &path).await })
    }
}

/// What an earlier call answered, which later calls cut pieces from
#[cfg(not(target_arch = "wasm32"))]
trait Source {
    /// The bytes of the piece at `path` and the schema of what they hold
    fn piece(
        &self,
        path: &[u32],
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(Vec<u8>, u64), Error>> + '_>>;
}

#[cfg(not(target_arch = "wasm32"))]
async fn read<F: Bits>(source: &dyn Source, path: &[u32]) -> Result<F, Error> {
    let (bytes, schema) = source.piece(path).await?;
    if schema != F::SCHEMA {
        return Err(Error::Outdated);
    }
    let mut r = yawn_bits::Reader::new(&bytes);
    let value = F::get(&mut r);
    if r.failed() {
        return Err(Error::Body("a piece of an answer is not readable".into()));
    }
    Ok(value)
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
    shared: std::rc::Rc<Shared<T>>,
}

/// The call of a `Pending` on the server, started by the first that polls it and shared by every
/// piece taken from it
#[cfg(not(target_arch = "wasm32"))]
struct Shared<T> {
    state: std::cell::RefCell<State<T>>,
    waiting: std::cell::RefCell<Vec<std::task::Waker>>,
}

#[cfg(not(target_arch = "wasm32"))]
enum State<T> {
    Running(std::pin::Pin<Box<dyn Future<Output = Result<T, Error>>>>),
    Done(Result<T, Error>),
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> Shared<T> {
    /// Drives the call; ready when it is done, and then everyone that waited is woken
    fn drive(&self, cx: &mut std::task::Context<'_>) -> std::task::Poll<()> {
        let mut state = self.state.borrow_mut();
        let State::Running(f) = &mut *state else { return std::task::Poll::Ready(()) };
        match f.as_mut().poll(cx) {
            std::task::Poll::Pending => {
                let mut waiting = self.waiting.borrow_mut();
                if !waiting.iter().any(|w| w.will_wake(cx.waker())) {
                    waiting.push(cx.waker().clone());
                }
                std::task::Poll::Pending
            }
            std::task::Poll::Ready(out) => {
                *state = State::Done(out);
                drop(state);
                for w in self.waiting.borrow_mut().drain(..) {
                    w.wake();
                }
                std::task::Poll::Ready(())
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Bits> Source for Shared<T> {
    fn piece(
        &self,
        path: &[u32],
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(Vec<u8>, u64), Error>> + '_>> {
        let path = path.to_vec();
        Box::pin(std::future::poll_fn(move |cx| {
            if self.drive(cx).is_pending() {
                return std::task::Poll::Pending;
            }
            let state = self.state.borrow();
            let State::Done(done) = &*state else { return std::task::Poll::Pending };
            std::task::Poll::Ready(match done {
                Err(e) => Err(e.clone()),
                Ok(value) => {
                    let mut bytes = Vec::new();
                    match value.project(&path, &mut bytes) {
                        Some(schema) => Ok((bytes, schema)),
                        None => Err(Error::Status(424, "the answer has no such field".into())),
                    }
                }
            })
        }))
    }
}

impl<T: Pipe + 'static> Pending<T> {
    /// A field of the answer, to go where an argument of another call does
    pub fn pick<G>(&self, field: impl FnOnce(T::Fields) -> Field<T, G>) -> Piped<G> {
        Piped {
            #[cfg(target_arch = "wasm32")]
            call: self.inner.position(),
            #[cfg(target_arch = "wasm32")]
            epoch: self.inner.epoch(),
            path: field(T::fields()).path().to_vec(),
            #[cfg(not(target_arch = "wasm32"))]
            source: self.shared.clone(),
            marker: PhantomData,
        }
    }
}

impl<T: Bits + 'static> IntoArg<T> for &Pending<T> {
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
    fn value(self) -> ArgFuture<T> {
        let source = self.shared.clone();
        Box::pin(async move { read::<T>(&*source, &[]).await })
    }
}

#[cfg(target_arch = "wasm32")]
impl<T: Bits> Future for Pending<T> {
    type Output = Result<T, Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.inner).poll(cx)
    }
}

/// The answer is shared with whoever took a piece of it, so what comes out of the future is a copy
#[cfg(not(target_arch = "wasm32"))]
impl<T: Bits + 'static> Future for Pending<T> {
    type Output = Result<T, Error>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.shared.drive(cx).is_pending() {
            return std::task::Poll::Pending;
        }
        let state = self.shared.state.borrow();
        let State::Done(done) = &*state else { return std::task::Poll::Pending };
        std::task::Poll::Ready(match done {
            Err(e) => Err(e.clone()),
            Ok(value) => {
                let mut bytes = Vec::new();
                value.put(&mut bytes);
                let mut r = yawn_bits::Reader::new(&bytes);
                let copy = T::get(&mut r);
                if r.failed() {
                    Err(Error::Body("the answer is not readable".into()))
                } else {
                    Ok(copy)
                }
            }
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn pending<T: Bits + 'static>(
    f: impl Future<Output = Result<T, Error>> + 'static,
) -> Pending<T> {
    Pending {
        shared: std::rc::Rc::new(Shared {
            state: std::cell::RefCell::new(State::Running(Box::pin(f))),
            waiting: Default::default(),
        }),
    }
}
