//! How the arguments and the answer of a server function are written on the wire. `Bits` is the
//! default: positional, small, bounded and checked against a hash of the type. `Json` is readable
//! by anything. `Postcard` and `Dynamic` are for types `Bits` cannot write (a `serde` type from
//! another crate, `flatten`, `untagged`).

/// Why bytes could not be turned into a value
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes were written for another version of the type
    Schema,
    Malformed(String),
}

/// An encoding of values of type `T`. A server function names one by `#[post(encoding = json)]`.
pub trait Codec<T> {
    /// The `content-type` of a body in this encoding: a call that is not a `GET` carries it, and a
    /// page of another site cannot send it without the server's consent
    const CONTENT_TYPE: &'static str;

    fn encode(value: &T) -> Result<Vec<u8>, String>;

    fn decode(bytes: &[u8]) -> Result<T, DecodeError>;
}

/// `#[derive(Bits)]` types (and the integers, strings, `Vec`s, `Option`s... they are made of)
pub struct Bits;

impl<T: yawn_bits::Bits> Codec<T> for Bits {
    const CONTENT_TYPE: &'static str = "application/vnd.yawn.bits";

    fn encode(value: &T) -> Result<Vec<u8>, String> {
        Ok(yawn_bits::to_vec(value))
    }

    fn decode(bytes: &[u8]) -> Result<T, DecodeError> {
        yawn_bits::from_slice(bytes).map_err(|e| match e {
            yawn_bits::Error::Schema => DecodeError::Schema,
            yawn_bits::Error::Malformed(why) => DecodeError::Malformed(why.into()),
        })
    }
}

/// `serde` types as JSON
#[cfg(any(feature = "json", feature = "server"))]
pub struct Json;

#[cfg(any(feature = "json", feature = "server"))]
impl<T: serde::Serialize + serde::de::DeserializeOwned> Codec<T> for Json {
    const CONTENT_TYPE: &'static str = "application/json";

    fn encode(value: &T) -> Result<Vec<u8>, String> {
        serde_json::to_vec(value).map_err(|e| e.to_string())
    }

    fn decode(bytes: &[u8]) -> Result<T, DecodeError> {
        serde_json::from_slice(bytes).map_err(|e| DecodeError::Malformed(e.to_string()))
    }
}

/// `serde` types as postcard: as small as `Bits`, with no type hash and no limit on nesting
#[cfg(feature = "postcard")]
pub struct Postcard;

#[cfg(feature = "postcard")]
impl<T: serde::Serialize + serde::de::DeserializeOwned> Codec<T> for Postcard {
    const CONTENT_TYPE: &'static str = "application/vnd.yawn.postcard";

    fn encode(value: &T) -> Result<Vec<u8>, String> {
        postcard::to_allocvec(value).map_err(|e| e.to_string())
    }

    fn decode(bytes: &[u8]) -> Result<T, DecodeError> {
        postcard::from_bytes(bytes).map_err(|e| DecodeError::Malformed(e.to_string()))
    }
}

/// `serde` types in a self-describing binary format, bounded like `Bits`
#[cfg(feature = "dynamic")]
pub struct Dynamic;

#[cfg(feature = "dynamic")]
impl<T: serde::Serialize + serde::de::DeserializeOwned> Codec<T> for Dynamic {
    const CONTENT_TYPE: &'static str = "application/vnd.yawn.dynamic";

    fn encode(value: &T) -> Result<Vec<u8>, String> {
        yawn_bits::dynamic::to_vec(value).map_err(|e| e.to_string())
    }

    fn decode(bytes: &[u8]) -> Result<T, DecodeError> {
        yawn_bits::dynamic::from_slice(bytes).map_err(|e| DecodeError::Malformed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round<C: Codec<T>, T: PartialEq + std::fmt::Debug>(v: T) {
        let bytes = C::encode(&v).unwrap();
        assert_eq!(C::decode(&bytes), Ok(v));
    }

    #[test]
    fn every_codec_reads_what_it_wrote() {
        round::<Bits, _>((1u8, "a".to_string(), vec![Some(2i64), None]));
        #[cfg(any(feature = "json", feature = "server"))]
        round::<Json, _>((1u8, "a".to_string(), vec![Some(2i64), None]));
        #[cfg(feature = "postcard")]
        round::<Postcard, _>((1u8, "a".to_string(), vec![Some(2i64), None]));
        #[cfg(feature = "dynamic")]
        round::<Dynamic, _>((1u8, "a".to_string(), vec![Some(2i64), None]));
    }

    #[test]
    fn bits_tells_a_changed_type_from_a_broken_message() {
        let bytes = Bits::encode(&(1u8, 2u8)).unwrap();
        assert_eq!(<Bits as Codec<(u16, u16)>>::decode(&bytes), Err(DecodeError::Schema));
        let mut cut = Bits::encode(&(1u8, 2u8)).unwrap();
        cut.pop();
        assert!(matches!(<Bits as Codec<(u8, u8)>>::decode(&cut), Err(DecodeError::Malformed(_))));
    }
}
