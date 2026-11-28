//! A self-describing binary format over serde, for the types `Bits` cannot write: `flatten`,
//! `untagged`, `serde_json::Value`, a field that is skipped when empty. Every value carries its
//! type tag and a field name is written once per message, so it is bigger and slower than `Bits`;
//! it is as bounded (nesting depth, lengths against the input).

use serde::de::{self, DeserializeSeed, Visitor};
use serde::ser;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_DEPTH: u32 = 96;

const NIL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const UINT: u8 = 3;
const NEG: u8 = 4;
const F32: u8 = 5;
const F64: u8 = 6;
const STR: u8 = 7;
const BYTES: u8 = 8;
const SEQ: u8 = 9;
const MAP: u8 = 10;
const SOME: u8 = 11;
const KEY_REF: u8 = 12;
const KEY_DEF: u8 = 13;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl ser::Error for Error {
    fn custom<T: fmt::Display>(m: T) -> Self {
        Error(m.to_string())
    }
}
impl de::Error for Error {
    fn custom<T: fmt::Display>(m: T) -> Self {
        Error(m.to_string())
    }
}

type Res<T> = Result<T, Error>;

fn err<T>(m: &str) -> Res<T> {
    Err(Error(m.to_string()))
}

pub fn to_vec<T: Serialize>(v: &T) -> Res<Vec<u8>> {
    let mut s = Ser { out: Vec::new(), keys: Vec::new() };
    v.serialize(&mut s)?;
    Ok(s.out)
}

pub fn from_slice<'a, T: Deserialize<'a>>(b: &'a [u8]) -> Res<T> {
    let mut d = De { buf: b, pos: 0, depth: 0, keys: Vec::new() };
    let v = T::deserialize(&mut d)?;
    if d.pos != b.len() {
        return err("trailing bytes");
    }
    Ok(v)
}

struct Ser {
    out: Vec<u8>,
    keys: Vec<&'static str>,
}

impl Ser {
    fn varint(&mut self, mut n: u64) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                self.out.push(b);
                return;
            }
            self.out.push(b | 0x80);
        }
    }
    fn key(&mut self, k: &'static str) {
        if let Some(i) = self.keys.iter().position(|x| core::ptr::eq(x.as_ptr(), k.as_ptr())) {
            self.out.push(KEY_REF);
            self.varint(i as u64);
        } else {
            self.keys.push(k);
            self.out.push(KEY_DEF);
            self.varint(k.len() as u64);
            self.out.extend_from_slice(k.as_bytes());
        }
    }
    fn splice_count(&mut self, at: usize, n: u64) {
        if n < 0x80 {
            self.out[at] = n as u8;
            return;
        }
        let mut tmp = [0u8; 10];
        let mut len = 0;
        let mut n = n;
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                tmp[len] = b;
                len += 1;
                break;
            }
            tmp[len] = b | 0x80;
            len += 1;
        }
        self.out[at] = tmp[0];
        self.out.splice(at + 1..at + 1, tmp[1..len].iter().copied()).for_each(drop);
    }
}

pub struct Compound<'a> {
    ser: &'a mut Ser,
    at: usize,
    count: u64,
}

impl<'a> Compound<'a> {
    fn open(ser: &'a mut Ser, tag: u8) -> Self {
        ser.out.push(tag);
        let at = ser.out.len();
        ser.out.push(0);
        Compound { ser, at, count: 0 }
    }
    fn close(self) -> Res<()> {
        self.ser.splice_count(self.at, self.count);
        Ok(())
    }
}

impl<'a> ser::Serializer for &'a mut Ser {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;

    fn is_human_readable(&self) -> bool {
        false
    }
    fn serialize_bool(self, v: bool) -> Res<()> {
        self.out.push(if v { TRUE } else { FALSE });
        Ok(())
    }
    fn serialize_i8(self, v: i8) -> Res<()> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i16(self, v: i16) -> Res<()> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i32(self, v: i32) -> Res<()> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i64(self, v: i64) -> Res<()> {
        if v >= 0 {
            self.out.push(UINT);
            self.varint(v as u64);
        } else {
            self.out.push(NEG);
            self.varint((-(v + 1)) as u64);
        }
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Res<()> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u16(self, v: u16) -> Res<()> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u32(self, v: u32) -> Res<()> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u64(self, v: u64) -> Res<()> {
        self.out.push(UINT);
        self.varint(v);
        Ok(())
    }
    fn serialize_f32(self, v: f32) -> Res<()> {
        self.out.push(F32);
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Res<()> {
        self.out.push(F64);
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }
    fn serialize_char(self, v: char) -> Res<()> {
        let mut b = [0u8; 4];
        self.serialize_str(v.encode_utf8(&mut b))
    }
    fn serialize_str(self, v: &str) -> Res<()> {
        self.out.push(STR);
        self.varint(v.len() as u64);
        self.out.extend_from_slice(v.as_bytes());
        Ok(())
    }
    fn serialize_bytes(self, v: &[u8]) -> Res<()> {
        self.out.push(BYTES);
        self.varint(v.len() as u64);
        self.out.extend_from_slice(v);
        Ok(())
    }
    fn serialize_none(self) -> Res<()> {
        self.out.push(NIL);
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Res<()> {
        self.out.push(SOME);
        v.serialize(self)
    }
    fn serialize_unit(self) -> Res<()> {
        self.out.push(NIL);
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Res<()> {
        self.out.push(NIL);
        Ok(())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Res<()> {
        self.key(variant);
        Ok(())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, v: &T) -> Res<()> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Res<()> {
        self.out.push(MAP);
        self.out.push(1);
        self.key(variant);
        v.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Res<Compound<'a>> {
        Ok(Compound::open(self, SEQ))
    }
    fn serialize_tuple(self, _: usize) -> Res<Compound<'a>> {
        Ok(Compound::open(self, SEQ))
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Res<Compound<'a>> {
        Ok(Compound::open(self, SEQ))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Res<Compound<'a>> {
        self.out.push(MAP);
        self.out.push(1);
        self.key(variant);
        Ok(Compound::open(self, SEQ))
    }
    fn serialize_map(self, _: Option<usize>) -> Res<Compound<'a>> {
        Ok(Compound::open(self, MAP))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Res<Compound<'a>> {
        Ok(Compound::open(self, MAP))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Res<Compound<'a>> {
        self.out.push(MAP);
        self.out.push(1);
        self.key(variant);
        Ok(Compound::open(self, MAP))
    }
}

impl<'a> ser::SerializeSeq for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Res<()> {
        self.count += 1;
        v.serialize(&mut *self.ser)
    }
    fn end(self) -> Res<()> {
        self.close()
    }
}
impl<'a> ser::SerializeTuple for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Res<()> {
        self.count += 1;
        v.serialize(&mut *self.ser)
    }
    fn end(self) -> Res<()> {
        self.close()
    }
}
impl<'a> ser::SerializeTupleStruct for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Res<()> {
        self.count += 1;
        v.serialize(&mut *self.ser)
    }
    fn end(self) -> Res<()> {
        self.close()
    }
}
impl<'a> ser::SerializeTupleVariant for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Res<()> {
        self.count += 1;
        v.serialize(&mut *self.ser)
    }
    fn end(self) -> Res<()> {
        self.close()
    }
}
impl<'a> ser::SerializeMap for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, k: &T) -> Res<()> {
        self.count += 1;
        k.serialize(&mut *self.ser)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> Res<()> {
        v.serialize(&mut *self.ser)
    }
    fn end(self) -> Res<()> {
        self.close()
    }
}
impl<'a> ser::SerializeStruct for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, k: &'static str, v: &T) -> Res<()> {
        self.count += 1;
        self.ser.key(k);
        v.serialize(&mut *self.ser)
    }
    fn end(self) -> Res<()> {
        self.close()
    }
}
impl<'a> ser::SerializeStructVariant for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, k: &'static str, v: &T) -> Res<()> {
        self.count += 1;
        self.ser.key(k);
        v.serialize(&mut *self.ser)
    }
    fn end(self) -> Res<()> {
        self.close()
    }
}

struct De<'de> {
    buf: &'de [u8],
    pos: usize,
    depth: u32,
    keys: Vec<&'de str>,
}

impl<'de> De<'de> {
    fn peek(&self) -> Res<u8> {
        self.buf.get(self.pos).copied().map_or_else(|| err("unexpected end"), Ok)
    }
    fn byte(&mut self) -> Res<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Ok(b)
    }
    fn take(&mut self, n: usize) -> Res<&'de [u8]> {
        if n > self.buf.len() - self.pos {
            return err("unexpected end");
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn varint(&mut self) -> Res<u64> {
        let mut n = 0u64;
        for shift in (0..70).step_by(7) {
            let b = self.byte()?;
            if shift == 63 && b > 1 {
                return err("varint overflow");
            }
            n |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(n);
            }
        }
        err("varint too long")
    }
    fn len(&mut self, per: usize) -> Res<usize> {
        let n = self.varint()?;
        let left = (self.buf.len() - self.pos) as u64;
        if n.checked_mul(per as u64).is_none_or(|need| need > left) {
            return err("length exceeds input");
        }
        Ok(n as usize)
    }
    fn str(&mut self) -> Res<&'de str> {
        let n = self.len(1)?;
        std::str::from_utf8(self.take(n)?).map_err(|_| Error("invalid utf-8".into()))
    }
    fn enter(&mut self) -> Res<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return err("nesting too deep");
        }
        Ok(())
    }
}

enum Tok<'de> {
    Nil,
    Bool(bool),
    U(u64),
    I(i64),
    F32(f32),
    F64(f64),
    Str(&'de str),
    Bytes(&'de [u8]),
    Seq(usize),
    Map(usize),
    Some,
}

impl<'de> De<'de> {
    #[inline(never)]
    fn token(&mut self) -> Res<Tok<'de>> {
        Ok(match self.byte()? {
            NIL => Tok::Nil,
            FALSE => Tok::Bool(false),
            TRUE => Tok::Bool(true),
            UINT => Tok::U(self.varint()?),
            NEG => {
                let n = self.varint()?;
                if n > i64::MAX as u64 {
                    return err("integer out of range");
                }
                Tok::I(-(n as i64) - 1)
            }
            F32 => Tok::F32(f32::from_le_bytes(self.take(4)?.try_into().unwrap())),
            F64 => Tok::F64(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            STR => Tok::Str(self.str()?),
            BYTES => {
                let n = self.len(1)?;
                Tok::Bytes(self.take(n)?)
            }
            SEQ => Tok::Seq(self.len(1)?),
            MAP => Tok::Map(self.len(2)?),
            SOME => Tok::Some,
            KEY_REF => {
                let i = self.varint()? as usize;
                match self.keys.get(i) {
                    Some(k) => Tok::Str(k),
                    None => return err("unknown key reference"),
                }
            }
            KEY_DEF => {
                let s = self.str()?;
                self.keys.push(s);
                Tok::Str(s)
            }
            _ => return err("unknown tag"),
        })
    }
}

struct Counted<'a, 'de> {
    de: &'a mut De<'de>,
    left: usize,
}

impl<'a, 'de> de::SeqAccess<'de> for Counted<'a, 'de> {
    type Error = Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Res<Option<T::Value>> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        seed.deserialize(&mut *self.de).map(Some)
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.left)
    }
}

impl<'a, 'de> de::MapAccess<'de> for Counted<'a, 'de> {
    type Error = Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Res<Option<K::Value>> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        seed.deserialize(&mut *self.de).map(Some)
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Res<V::Value> {
        seed.deserialize(&mut *self.de)
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.left)
    }
}

struct Variant<'a, 'de> {
    de: &'a mut De<'de>,
    content: bool,
}

impl<'a, 'de> de::EnumAccess<'de> for Variant<'a, 'de> {
    type Error = Error;
    type Variant = Self;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Res<(V::Value, Self)> {
        let v = seed.deserialize(&mut *self.de)?;
        Ok((v, self))
    }
}

impl<'a, 'de> de::VariantAccess<'de> for Variant<'a, 'de> {
    type Error = Error;
    fn unit_variant(self) -> Res<()> {
        if self.content {
            de::Deserialize::deserialize(&mut *self.de).map(|_: de::IgnoredAny| ())?;
        }
        Ok(())
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Res<T::Value> {
        if !self.content {
            return err("expected variant content");
        }
        seed.deserialize(&mut *self.de)
    }
    fn tuple_variant<V: Visitor<'de>>(self, _: usize, v: V) -> Res<V::Value> {
        if !self.content {
            return err("expected variant content");
        }
        de::Deserializer::deserialize_seq(&mut *self.de, v)
    }
    fn struct_variant<V: Visitor<'de>>(self, _: &'static [&'static str], v: V) -> Res<V::Value> {
        if !self.content {
            return err("expected variant content");
        }
        de::Deserializer::deserialize_map(&mut *self.de, v)
    }
}

impl<'de> de::Deserializer<'de> for &mut De<'de> {
    type Error = Error;

    fn is_human_readable(&self) -> bool {
        false
    }

    fn deserialize_any<V: Visitor<'de>>(self, v: V) -> Res<V::Value> {
        match self.token()? {
            Tok::Nil => v.visit_unit(),
            Tok::Bool(b) => v.visit_bool(b),
            Tok::U(n) => v.visit_u64(n),
            Tok::I(n) => v.visit_i64(n),
            Tok::F32(n) => v.visit_f32(n),
            Tok::F64(n) => v.visit_f64(n),
            Tok::Str(s) => v.visit_borrowed_str(s),
            Tok::Bytes(b) => v.visit_borrowed_bytes(b),
            Tok::Seq(n) => {
                self.enter()?;
                let r = v.visit_seq(Counted { de: &mut *self, left: n });
                self.depth -= 1;
                r
            }
            Tok::Map(n) => {
                self.enter()?;
                let r = v.visit_map(Counted { de: &mut *self, left: n });
                self.depth -= 1;
                r
            }
            Tok::Some => {
                self.enter()?;
                let r = v.visit_some(&mut *self);
                self.depth -= 1;
                r
            }
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Res<V::Value> {
        match self.peek()? {
            NIL => {
                self.pos += 1;
                v.visit_none()
            }
            SOME => {
                self.pos += 1;
                self.enter()?;
                let r = v.visit_some(&mut *self);
                self.depth -= 1;
                r
            }
            _ => err("expected option"),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _: &'static str, v: V) -> Res<V::Value> {
        v.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        v: V,
    ) -> Res<V::Value> {
        match self.peek()? {
            STR | KEY_REF | KEY_DEF => v.visit_enum(Variant { de: self, content: false }),
            MAP => {
                self.pos += 1;
                if self.len(2)? != 1 {
                    return err("enum map must have one entry");
                }
                self.enter()?;
                let r = v.visit_enum(Variant { de: &mut *self, content: true });
                self.depth -= 1;
                r
            }
            _ => err("expected enum"),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}
