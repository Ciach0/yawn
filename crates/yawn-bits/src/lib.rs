//! Yawn's binary wire format. A value is written field after field in the order of its
//! declaration, with no names and no tags beyond the variant of an enum, so it is as small as the
//! data allows and a decoder is straight-line code. A message starts with a version byte and a
//! hash of the type, so a client older than the server finds out instead of reading garbage, and
//! the decoder is bounded: nesting depth and every length it reads are checked, so a hostile
//! message cannot overflow the stack or ask for memory it does not carry.
//!
//! ```
//! use yawn_bits::Bits;
//!
//! #[derive(Bits, PartialEq, Debug)]
//! #[bits(crate = "yawn_bits")]
//! struct Todo {
//!     id: u64,
//!     title: String,
//!     tags: Vec<String>,
//! }
//!
//! let todo = Todo { id: 7, title: "milk".into(), tags: vec!["shop".into()] };
//! let bytes = yawn_bits::to_vec(&todo);
//! assert_eq!(yawn_bits::from_slice::<Todo>(&bytes), Ok(todo));
//! ```

mod impls;
pub mod schema;

#[cfg(feature = "serde")]
pub mod dynamic;

pub use yawn_bits_derive::Bits;

use std::fmt;

/// How deep values may nest in a message
pub const MAX_DEPTH: u32 = 64;

/// The most a collection reserves before its elements have actually been read
const PREALLOC: usize = 1024;

const VERSION: u8 = 1;

/// Why a message could not be read
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The message was written for another shape of the type: the sender is a different version
    Schema,
    /// The message is not a valid encoding of the type
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Schema => f.write_str("the message was written for another version of the type"),
            Error::Malformed(why) => write!(f, "malformed message: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// A type with a binary encoding. Derive it with `#[derive(Bits)]`.
pub trait Bits: Sized {
    /// A hash of the shape of the type: names and types of its fields and variants. Changing the
    /// shape changes the hash, and a message of another hash is refused.
    const SCHEMA: u64;

    fn put(&self, w: &mut Vec<u8>);

    /// Reads a value. A failure is recorded in the reader, which from then on yields empty values
    /// (zeros, empty strings and collections) without reading more, so the caller checks once at
    /// the end.
    fn get(r: &mut Reader) -> Self;

    #[doc(hidden)]
    fn put_slice(items: &[Self], w: &mut Vec<u8>) {
        for item in items {
            item.put(w);
        }
    }

    #[doc(hidden)]
    fn get_vec(r: &mut Reader, n: usize) -> Vec<Self> {
        let mut out = Vec::with_capacity(n.min(PREALLOC));
        for _ in 0..n {
            out.push(Self::get(r));
        }
        out
    }
}

/// The input of a decoder
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    depth: u32,
    err: Option<&'static str>,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0, depth: 0, err: None }
    }

    #[cold]
    #[inline(never)]
    pub fn fail(&mut self, why: &'static str) {
        if self.err.is_none() {
            self.err = Some(why);
        }
        self.pos = self.buf.len();
    }

    pub fn failed(&self) -> bool {
        self.err.is_some()
    }

    #[inline(never)]
    pub fn byte(&mut self) -> u8 {
        match self.buf.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                b
            }
            None => {
                self.fail("unexpected end");
                0
            }
        }
    }

    #[inline(never)]
    pub fn take(&mut self, n: usize) -> &'a [u8] {
        if n > self.buf.len() - self.pos {
            self.fail("unexpected end");
            return &[];
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        s
    }

    #[inline(never)]
    pub fn varint(&mut self) -> u64 {
        let mut n = 0u64;
        let mut shift = 0;
        loop {
            let b = self.byte();
            if shift == 63 && b > 1 {
                self.fail("number too large");
                return 0;
            }
            n |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return n;
            }
            shift += 7;
            if shift > 63 {
                self.fail("number too large");
                return 0;
            }
        }
    }

    /// A length of `per` bytes at least each, which the input must still be long enough to hold
    #[inline(never)]
    pub fn len(&mut self, per: usize) -> usize {
        let n = self.varint();
        if n.checked_mul(per as u64).is_none_or(|need| need > (self.buf.len() - self.pos) as u64) {
            self.fail("length exceeds the message");
            return 0;
        }
        n as usize
    }

    /// Goes one level deeper; false (and a failed reader) past [`MAX_DEPTH`]
    pub fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.fail("nested too deep");
            return false;
        }
        true
    }

    pub fn leave(&mut self) {
        self.depth -= 1;
    }

    fn done(self) -> Result<(), Error> {
        match self.err {
            Some(why) => Err(Error::Malformed(why)),
            None if self.pos == self.buf.len() => Ok(()),
            None => Err(Error::Malformed("bytes after the value")),
        }
    }
}

#[inline(never)]
pub fn put_varint(w: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        w.push((n as u8) | 0x80);
        n >>= 7;
    }
    w.push(n as u8);
}

fn fold(schema: u64) -> [u8; 4] {
    ((schema ^ (schema >> 32)) as u32).to_le_bytes()
}

/// A message: the version, the hash of `T`, the value
pub fn to_vec<T: Bits>(v: &T) -> Vec<u8> {
    let mut w = Vec::with_capacity(64);
    w.push(VERSION);
    w.extend_from_slice(&fold(T::SCHEMA));
    v.put(&mut w);
    w
}

pub fn from_slice<T: Bits>(b: &[u8]) -> Result<T, Error> {
    if b.len() < 5 || b[0] != VERSION || b[1..5] != fold(T::SCHEMA) {
        return Err(Error::Schema);
    }
    let mut r = Reader::new(&b[5..]);
    let v = T::get(&mut r);
    r.done()?;
    Ok(v)
}

/// A value without the header, for a message that carries several (the header of the whole is
/// written once)
#[doc(hidden)]
pub fn put_value<T: Bits>(v: &T, w: &mut Vec<u8>) {
    v.put(w);
}

#[doc(hidden)]
pub fn get_value<T: Bits>(r: &mut Reader) -> T {
    T::get(r)
}

#[doc(hidden)]
pub fn finish(r: Reader) -> Result<(), Error> {
    r.done()
}
