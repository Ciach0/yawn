use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use yawn_bits::{Bits, Error, from_slice, to_vec};

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct Rule {
    channel: u64,
    name: String,
    enabled: bool,
    cooldown: u32,
    message: Option<String>,
}

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct Config {
    lang: String,
    flags: Vec<bool>,
    limits: Vec<i32>,
    rules: Vec<Rule>,
    ratio: f64,
    pair: (u8, i64),
    grid: [u16; 3],
    unit: (),
}

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct Wrapper(u32, String);

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct Marker;

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
enum Shape {
    Empty,
    Circle(f32),
    Rect { w: u32, h: u32 },
    Pair(String, Vec<u8>),
}

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
enum Tree {
    Leaf,
    Node(Box<Tree>, Box<Tree>),
}

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct Dir {
    name: String,
    #[bits(opaque)]
    children: Vec<Dir>,
}

fn rule(i: u64) -> Rule {
    Rule {
        channel: 900_000_000_000_000_000 + i * 104_729,
        name: format!("rule-{i}"),
        enabled: i.is_multiple_of(2),
        cooldown: (i as u32) * 30,
        message: i.is_multiple_of(3).then(|| format!("hello {i}")),
    }
}

fn config() -> Config {
    Config {
        lang: "pl".into(),
        flags: (0..40).map(|i| i % 3 != 0).collect(),
        limits: (-20..20).map(|i| i * 37).collect(),
        rules: (0..50).map(rule).collect(),
        ratio: -0.25,
        pair: (255, i64::MIN),
        grid: [1, 2, 65535],
        unit: (),
    }
}

fn round<T: Bits + PartialEq + std::fmt::Debug>(v: T) {
    let bytes = to_vec(&v);
    assert_eq!(from_slice::<T>(&bytes), Ok(v));
}

#[test]
fn values_come_back() {
    round(config());
    round(Wrapper(7, "x".into()));
    round(Marker);
    for s in [
        Shape::Empty,
        Shape::Circle(1.5),
        Shape::Rect { w: 3, h: 4 },
        Shape::Pair("a".into(), vec![0, 255, 7]),
    ] {
        round(s);
    }
    round(Tree::Node(
        Box::new(Tree::Leaf),
        Box::new(Tree::Node(Box::new(Tree::Leaf), Box::new(Tree::Leaf))),
    ));
    round(Dir { name: "a".into(), children: vec![Dir { name: "b".into(), children: vec![] }] });
    round((1u8, -2i16, 3u32, -4i64, 5usize, -6isize, u128::MAX, i128::MIN));
    round(('ż', true, "s".to_string()));
    round(vec![u64::MAX, 0, 1 << 40]);
    round(vec![i64::MIN, -1, 0, 1, i64::MAX]);
    round(Some(Some(3u8)));
    round(Some(Option::<u8>::None));
    round(Ok::<u8, String>(3));
    round(Err::<u8, String>("no".into()));
    round(Box::new(5u8));
    round(BTreeMap::from([(1u8, "a".to_string()), (2, "b".to_string())]));
    round(BTreeSet::from([3u32, 1, 2]));
    round(HashMap::from([("k".to_string(), vec![1u8, 2])]));
    round(HashSet::from([1u16, 2, 3]));
    round(f64::MAX);
    round(vec![String::new(), "é".into()]);
}

#[test]
fn the_encoding_is_small_and_positional() {
    assert_eq!(to_vec(&Marker).len(), 5);
    let bytes = to_vec(&Wrapper(7, "ab".into()));
    assert_eq!(&bytes[5..], &[7, 2, b'a', b'b']);
    assert_eq!(&to_vec(&Shape::Rect { w: 3, h: 4 })[5..], &[2, 3, 4]);
    assert_eq!(&to_vec(&vec![1u8, 2, 3])[5..], &[3, 1, 2, 3]);
}

#[derive(Bits, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct Old {
    a: u32,
}

#[derive(Bits, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct New {
    a: u32,
    b: u32,
}

#[test]
fn another_shape_is_refused() {
    let old = to_vec(&Old { a: 1 });
    assert_eq!(from_slice::<New>(&old), Err(Error::Schema));
    assert_ne!(Old::SCHEMA, New::SCHEMA);
    assert_eq!(from_slice::<u32>(&[]), Err(Error::Schema));
    let mut other_version = to_vec(&1u32);
    other_version[0] = 9;
    assert_eq!(from_slice::<u32>(&other_version), Err(Error::Schema));
    assert_ne!(Vec::<u32>::SCHEMA, Vec::<u64>::SCHEMA);
    assert_ne!(Shape::SCHEMA, Tree::SCHEMA);
}

fn deep(n: usize) -> Vec<u8> {
    let mut v = to_vec(&Tree::Leaf);
    v.truncate(5);
    v.extend(std::iter::repeat_n(1u8, n));
    v.extend(std::iter::repeat_n(0u8, n + 1));
    v
}

#[test]
fn hostile_input_is_refused() {
    let r = from_slice::<Tree>(&deep(1_000_000));
    assert_eq!(r, Err(Error::Malformed("nested too deep")));
    assert!(from_slice::<Tree>(&deep(20)).is_ok());

    let mut bomb = to_vec(&Vec::<u64>::new());
    bomb.truncate(5);
    yawn_bits::put_varint(&mut bomb, 1 << 40);
    bomb.extend_from_slice(&[0; 8]);
    assert_eq!(from_slice::<Vec<u64>>(&bomb), Err(Error::Malformed("length exceeds the message")));

    let mut huge_string = to_vec(&String::new());
    huge_string.truncate(5);
    yawn_bits::put_varint(&mut huge_string, u64::MAX);
    assert!(from_slice::<String>(&huge_string).is_err());

    let mut bad_bool = to_vec(&true);
    *bad_bool.last_mut().unwrap() = 7;
    assert_eq!(from_slice::<bool>(&bad_bool), Err(Error::Malformed("not a bool")));

    let mut bad_variant = to_vec(&Shape::Empty);
    *bad_variant.last_mut().unwrap() = 99;
    assert_eq!(from_slice::<Shape>(&bad_variant), Err(Error::Malformed("not a variant")));

    let mut trailing = to_vec(&1u8);
    trailing.push(0);
    assert!(from_slice::<u8>(&trailing).is_err());

    let mut bad_utf8 = to_vec(&"ab".to_string());
    *bad_utf8.last_mut().unwrap() = 0xff;
    assert_eq!(from_slice::<String>(&bad_utf8), Err(Error::Malformed("not utf-8")));

    let mut wide = to_vec(&0u8);
    wide.truncate(5);
    wide.extend_from_slice(&[0xff; 12]);
    assert!(from_slice::<u64>(&wide).is_err());
    let mut small = to_vec(&0u32);
    small.truncate(5);
    yawn_bits::put_varint(&mut small, 1 << 33);
    assert_eq!(from_slice::<u32>(&small), Err(Error::Malformed("number out of range")));
}

#[test]
fn corrupt_input_never_panics() {
    let good = to_vec(&config());
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let (mut ok, mut err) = (0, 0);
    for i in 0..40_000 {
        let mut b = if i % 2 == 0 {
            good.clone()
        } else {
            (0..(next() % 400) as usize).map(|_| next() as u8).collect()
        };
        if i % 2 == 0 {
            for _ in 0..4 {
                let at = 5 + next() as usize % (b.len() - 5);
                b[at] = next() as u8;
            }
        } else if b.len() >= 5 {
            b[..5].copy_from_slice(&good[..5]);
        }
        match from_slice::<Config>(&b) {
            Ok(_) => ok += 1,
            Err(_) => err += 1,
        }
    }
    assert!(err > 0 && ok + err == 40_000);
    for cut in 0..good.len() {
        assert!(from_slice::<Config>(&good[..cut]).is_err());
    }
}

#[test]
fn a_recursive_type_through_a_vec_has_a_schema() {
    assert_eq!(Dir::SCHEMA, Dir::SCHEMA);
    round(Dir {
        name: "root".into(),
        children: (0..3).map(|i| Dir { name: i.to_string(), children: vec![] }).collect(),
    });
}

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct Profile {
    age: u8,
    name: String,
}

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct User {
    id: u64,
    profile: Profile,
    tags: Vec<String>,
}

#[derive(Bits, Clone, PartialEq, Debug)]
#[bits(crate = "yawn_bits")]
struct Pair(u32, String);

fn user() -> User {
    User {
        id: 77,
        profile: Profile { age: 30, name: "ann".into() },
        tags: vec!["a".into(), "b".into()],
    }
}

fn projected<T: Bits>(v: &T, path: &[u32]) -> Option<(u64, Vec<u8>)> {
    let mut w = Vec::new();
    v.project(path, &mut w).map(|schema| (schema, w))
}

#[test]
fn a_field_can_be_taken_from_a_value_by_its_path() {
    use yawn_bits::Pipe;
    let f = User::fields();
    assert_eq!(f.id.path(), &[0]);
    assert_eq!(f.profile.path(), &[1]);
    assert_eq!(f.tags.path(), &[2]);
    assert_eq!(Pair::fields().f1.path(), &[1]);
    let u = user();
    let (schema, bytes) = projected(&u, f.id.path()).unwrap();
    assert_eq!((schema, bytes), (u64::SCHEMA, to_vec(&77u64)[5..].to_vec()));
    let (schema, bytes) = projected(&u, f.profile.path()).unwrap();
    assert_eq!(schema, Profile::SCHEMA);
    assert_eq!(
        from_slice::<Profile>(&[&to_vec(&0u8)[..5], &bytes[..]].concat()),
        Err(Error::Schema)
    );
    let mut message = yawn_bits::header::<Profile>().to_vec();
    message.extend_from_slice(&bytes);
    assert_eq!(from_slice::<Profile>(&message), Ok(u.profile.clone()));
    let (schema, bytes) = projected(&u, &[1, 1]).unwrap();
    assert_eq!((schema, bytes), (String::SCHEMA, to_vec(&"ann".to_string())[5..].to_vec()));
    assert_eq!(projected(&u, &[]).map(|(s, _)| s), Some(User::SCHEMA));
    assert_eq!(projected(&u, &[9]), None);
    assert_eq!(projected(&u, &[0, 0]), None);
    assert_eq!(projected(&Pair(4, "x".into()), &[1]).map(|(_, b)| b), Some(vec![1, b'x']));
    assert_eq!(projected(&(1u8, "t".to_string()), &[1]).map(|(s, _)| s), Some(String::SCHEMA));
}

#[test]
fn a_message_can_be_put_together_from_parts() {
    let mut message = yawn_bits::header::<(u64, String)>().to_vec();
    message.extend_from_slice(&to_vec(&5u64)[5..]);
    message.extend_from_slice(&to_vec(&"hi".to_string())[5..]);
    assert_eq!(from_slice::<(u64, String)>(&message), Ok((5, "hi".to_string())));
    assert_eq!(message, to_vec(&(5u64, "hi".to_string())));
}
