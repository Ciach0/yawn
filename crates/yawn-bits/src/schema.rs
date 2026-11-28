//! The hash of a type's shape, computed in constants

const SEED: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

pub const fn text(s: &str) -> u64 {
    let b = s.as_bytes();
    let mut h = SEED;
    let mut i = 0;
    while i < b.len() {
        h ^= b[i] as u64;
        h = h.wrapping_mul(PRIME);
        i += 1;
    }
    h
}

pub const fn mix(a: u64, b: u64) -> u64 {
    let mut h = a ^ b.rotate_left(29);
    h = h.wrapping_mul(PRIME);
    h ^ (h >> 31)
}

/// The hash of something named made of named parts, each with the hash of its own type
pub const fn shape(name: &str, parts: &[(&str, u64)]) -> u64 {
    let mut h = text(name);
    let mut i = 0;
    while i < parts.len() {
        h = mix(h, text(parts[i].0));
        h = mix(h, parts[i].1);
        i += 1;
    }
    h
}
