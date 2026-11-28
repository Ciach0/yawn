use crate::schema::{mix, text};
use crate::{Bits, PREALLOC, Reader, put_varint};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, Hash};

impl Bits for u8 {
    const SCHEMA: u64 = text("u8");
    fn put(&self, w: &mut Vec<u8>) {
        w.push(*self);
    }
    #[inline(never)]
    fn get(r: &mut Reader) -> Self {
        r.byte()
    }
    fn put_slice(items: &[Self], w: &mut Vec<u8>) {
        w.extend_from_slice(items);
    }
    fn get_vec(r: &mut Reader, n: usize) -> Vec<Self> {
        r.take(n).to_vec()
    }
}

impl Bits for i8 {
    const SCHEMA: u64 = text("i8");
    fn put(&self, w: &mut Vec<u8>) {
        w.push(*self as u8);
    }
    #[inline(never)]
    fn get(r: &mut Reader) -> Self {
        r.byte() as i8
    }
}

macro_rules! unsigned {
    ($($t:ty),*) => {$(
        impl Bits for $t {
            const SCHEMA: u64 = text(stringify!($t));
            fn put(&self, w: &mut Vec<u8>) {
                put_varint(w, *self as u64);
            }
            #[inline(never)]
            fn get(r: &mut Reader) -> Self {
                let n = r.varint();
                match <$t>::try_from(n) {
                    Ok(v) => v,
                    Err(_) => {
                        r.fail("number out of range");
                        0
                    }
                }
            }
        }
    )*};
}
unsigned!(u16, u32, u64, usize);

macro_rules! signed {
    ($($t:ty),*) => {$(
        impl Bits for $t {
            const SCHEMA: u64 = text(stringify!($t));
            fn put(&self, w: &mut Vec<u8>) {
                let n = *self as i64;
                put_varint(w, ((n << 1) ^ (n >> 63)) as u64);
            }
            #[inline(never)]
            fn get(r: &mut Reader) -> Self {
                let z = r.varint();
                let n = ((z >> 1) as i64) ^ -((z & 1) as i64);
                match <$t>::try_from(n) {
                    Ok(v) => v,
                    Err(_) => {
                        r.fail("number out of range");
                        0
                    }
                }
            }
        }
    )*};
}
signed!(i16, i32, i64, isize);

macro_rules! wide {
    ($($t:ty),*) => {$(
        impl Bits for $t {
            const SCHEMA: u64 = text(stringify!($t));
            fn put(&self, w: &mut Vec<u8>) {
                w.extend_from_slice(&self.to_le_bytes());
            }
            fn get(r: &mut Reader) -> Self {
                let b = r.take(16);
                match <[u8; 16]>::try_from(b) {
                    Ok(b) => <$t>::from_le_bytes(b),
                    Err(_) => 0,
                }
            }
        }
    )*};
}
wide!(u128, i128);

macro_rules! float {
    ($($t:ty: $n:literal),*) => {$(
        impl Bits for $t {
            const SCHEMA: u64 = text(stringify!($t));
            fn put(&self, w: &mut Vec<u8>) {
                w.extend_from_slice(&self.to_le_bytes());
            }
            #[inline(never)]
            fn get(r: &mut Reader) -> Self {
                match <[u8; $n]>::try_from(r.take($n)) {
                    Ok(b) => <$t>::from_le_bytes(b),
                    Err(_) => 0.0,
                }
            }
        }
    )*};
}
float!(f32: 4, f64: 8);

impl Bits for bool {
    const SCHEMA: u64 = text("bool");
    fn put(&self, w: &mut Vec<u8>) {
        w.push(*self as u8);
    }
    #[inline(never)]
    fn get(r: &mut Reader) -> Self {
        match r.byte() {
            0 => false,
            1 => true,
            _ => {
                r.fail("not a bool");
                false
            }
        }
    }
}

impl Bits for char {
    const SCHEMA: u64 = text("char");
    fn put(&self, w: &mut Vec<u8>) {
        put_varint(w, *self as u64);
    }
    fn get(r: &mut Reader) -> Self {
        let n = r.varint();
        match u32::try_from(n).ok().and_then(char::from_u32) {
            Some(c) => c,
            None => {
                r.fail("not a char");
                '\0'
            }
        }
    }
}

impl Bits for () {
    const SCHEMA: u64 = text("()");
    fn put(&self, _: &mut Vec<u8>) {}
    fn get(_: &mut Reader) -> Self {}
}

impl Bits for String {
    const SCHEMA: u64 = text("String");
    #[inline(never)]
    fn put(&self, w: &mut Vec<u8>) {
        put_varint(w, self.len() as u64);
        w.extend_from_slice(self.as_bytes());
    }
    #[inline(never)]
    fn get(r: &mut Reader) -> Self {
        let n = r.len(1);
        match std::str::from_utf8(r.take(n)) {
            Ok(s) => s.to_owned(),
            Err(_) => {
                r.fail("not utf-8");
                String::new()
            }
        }
    }
}

impl<T: Bits> Bits for Option<T> {
    const SCHEMA: u64 = mix(text("Option"), T::SCHEMA);
    fn put(&self, w: &mut Vec<u8>) {
        match self {
            None => w.push(0),
            Some(v) => {
                w.push(1);
                v.put(w);
            }
        }
    }
    fn get(r: &mut Reader) -> Self {
        match r.byte() {
            0 => None,
            1 if r.enter() => {
                let v = T::get(r);
                r.leave();
                Some(v)
            }
            _ => {
                r.fail("not an option");
                None
            }
        }
    }
}

impl<T: Bits, E: Bits> Bits for Result<T, E> {
    const SCHEMA: u64 = mix(mix(text("Result"), T::SCHEMA), E::SCHEMA);
    fn put(&self, w: &mut Vec<u8>) {
        match self {
            Ok(v) => {
                w.push(0);
                v.put(w);
            }
            Err(e) => {
                w.push(1);
                e.put(w);
            }
        }
    }
    fn get(r: &mut Reader) -> Self {
        let tag = r.byte();
        if !r.enter() {
            return Ok(T::get(r));
        }
        let out = match tag {
            0 => Ok(T::get(r)),
            1 => Err(E::get(r)),
            _ => {
                r.fail("not a result");
                Ok(T::get(r))
            }
        };
        r.leave();
        out
    }
}

impl<T: Bits> Bits for Box<T> {
    const SCHEMA: u64 = mix(text("Box"), T::SCHEMA);
    fn put(&self, w: &mut Vec<u8>) {
        (**self).put(w);
    }
    fn project(&self, path: &[u32], w: &mut Vec<u8>) -> Option<u64> {
        (**self).project(path, w)
    }
    fn get(r: &mut Reader) -> Self {
        if !r.enter() {
            return Box::new(T::get(r));
        }
        let v = T::get(r);
        r.leave();
        Box::new(v)
    }
}

impl<T: Bits> Bits for Vec<T> {
    const SCHEMA: u64 = mix(text("Vec"), T::SCHEMA);
    fn put(&self, w: &mut Vec<u8>) {
        put_varint(w, self.len() as u64);
        T::put_slice(self, w);
    }
    fn get(r: &mut Reader) -> Self {
        let n = r.len(1);
        if !r.enter() {
            return Vec::new();
        }
        let out = T::get_vec(r, n);
        r.leave();
        out
    }
}

impl<T: Bits, const N: usize> Bits for [T; N] {
    const SCHEMA: u64 = mix(mix(text("array"), T::SCHEMA), N as u64);
    fn put(&self, w: &mut Vec<u8>) {
        for item in self {
            item.put(w);
        }
    }
    fn get(r: &mut Reader) -> Self {
        std::array::from_fn(|_| T::get(r))
    }
}

impl<T: Bits + Ord> Bits for BTreeSet<T> {
    const SCHEMA: u64 = mix(text("BTreeSet"), T::SCHEMA);
    fn put(&self, w: &mut Vec<u8>) {
        put_varint(w, self.len() as u64);
        for item in self {
            item.put(w);
        }
    }
    fn get(r: &mut Reader) -> Self {
        let n = r.len(1);
        let mut out = BTreeSet::new();
        if !r.enter() {
            return out;
        }
        for _ in 0..n {
            if r.failed() {
                break;
            }
            out.insert(T::get(r));
        }
        r.leave();
        out
    }
}

impl<K: Bits + Ord, V: Bits> Bits for BTreeMap<K, V> {
    const SCHEMA: u64 = mix(mix(text("BTreeMap"), K::SCHEMA), V::SCHEMA);
    fn put(&self, w: &mut Vec<u8>) {
        put_varint(w, self.len() as u64);
        for (k, v) in self {
            k.put(w);
            v.put(w);
        }
    }
    fn get(r: &mut Reader) -> Self {
        let n = r.len(1);
        let mut out = BTreeMap::new();
        if !r.enter() {
            return out;
        }
        for _ in 0..n {
            if r.failed() {
                break;
            }
            let k = K::get(r);
            let v = V::get(r);
            out.insert(k, v);
        }
        r.leave();
        out
    }
}

impl<T: Bits + Eq + Hash, S: BuildHasher + Default> Bits for HashSet<T, S> {
    const SCHEMA: u64 = mix(text("HashSet"), T::SCHEMA);
    fn put(&self, w: &mut Vec<u8>) {
        put_varint(w, self.len() as u64);
        for item in self {
            item.put(w);
        }
    }
    fn get(r: &mut Reader) -> Self {
        let n = r.len(1);
        let mut out = HashSet::with_capacity_and_hasher(n.min(PREALLOC), S::default());
        if !r.enter() {
            return out;
        }
        for _ in 0..n {
            if r.failed() {
                break;
            }
            out.insert(T::get(r));
        }
        r.leave();
        out
    }
}

impl<K: Bits + Eq + Hash, V: Bits, S: BuildHasher + Default> Bits for HashMap<K, V, S> {
    const SCHEMA: u64 = mix(mix(text("HashMap"), K::SCHEMA), V::SCHEMA);
    fn put(&self, w: &mut Vec<u8>) {
        put_varint(w, self.len() as u64);
        for (k, v) in self {
            k.put(w);
            v.put(w);
        }
    }
    fn get(r: &mut Reader) -> Self {
        let n = r.len(1);
        let mut out = HashMap::with_capacity_and_hasher(n.min(PREALLOC), S::default());
        if !r.enter() {
            return out;
        }
        for _ in 0..n {
            if r.failed() {
                break;
            }
            let k = K::get(r);
            let v = V::get(r);
            out.insert(k, v);
        }
        r.leave();
        out
    }
}

macro_rules! tuples {
    ($(($($t:ident $i:tt),+))*) => {$(
        impl<$($t: Bits),+> Bits for ($($t,)+) {
            const SCHEMA: u64 = {
                let mut h = text("tuple");
                $(h = mix(h, $t::SCHEMA);)+
                h
            };
            fn put(&self, w: &mut Vec<u8>) {
                $(self.$i.put(w);)+
            }
            fn project(&self, path: &[u32], w: &mut Vec<u8>) -> Option<u64> {
                match path.split_first() {
                    None => {
                        self.put(w);
                        Some(Self::SCHEMA)
                    }
                    $(Some((&$i, rest)) => self.$i.project(rest, w),)+
                    _ => None,
                }
            }
            fn get(r: &mut Reader) -> Self {
                ($($t::get(r),)+)
            }
        }
    )*};
}
tuples! {
    (A 0)
    (A 0, B 1)
    (A 0, B 1, C 2)
    (A 0, B 1, C 2, D 3)
    (A 0, B 1, C 2, D 3, E 4)
    (A 0, B 1, C 2, D 3, E 4, F 5)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7)
}
