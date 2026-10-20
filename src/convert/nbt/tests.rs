// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Tests for the NBT layer: ports of Chunker's tag, reader/writer and file tests, golden bytes, hostile input.

use super::file::{self, Compression};
use super::io::{self, Flavor, MAX_ARRAY_LENGTH, MAX_DEPTH, MAX_LIST_LENGTH};
use super::{Compound, FromTag, NbtError, Tag, TagType, compound};

const FLAVORS: [Flavor; 2] = [Flavor::Java, Flavor::Bedrock];

fn every_type() -> Vec<Tag> {
    vec![
        Tag::Byte(i8::MIN),
        Tag::Byte(0),
        Tag::Byte(i8::MAX),
        Tag::Short(i16::MIN),
        Tag::Short(1337),
        Tag::Int(i32::MIN),
        Tag::Int(i32::MAX),
        Tag::Int(-1337),
        Tag::Long(i64::MIN),
        Tag::Long(i64::MAX),
        Tag::Float(1.23),
        Tag::Float(f32::MAX),
        Tag::Float(f32::MIN_POSITIVE),
        Tag::Float(f32::NEG_INFINITY),
        Tag::Double(-0.5),
        Tag::Double(f64::MAX),
        Tag::Double(f64::INFINITY),
        Tag::ByteArray(vec![]),
        Tag::ByteArray(vec![i8::MIN, -1, 0, 1, i8::MAX]),
        Tag::String(String::new()),
        Tag::String("Hello".into()),
        Tag::String(" hello world!\n".into()),
        Tag::String("caf\u{e9} \u{1F60E} \u{4e16}\u{754c}".into()),
        Tag::String("nul\0inside".into()),
        Tag::List(vec![]),
        Tag::list([1i8, 2, 3]),
        Tag::list([1i16, -2]),
        Tag::list([1i32]),
        Tag::list([1i64, 2]),
        Tag::list([1.5f32]),
        Tag::list([1.5f64]),
        Tag::list(["a", "b"]),
        Tag::List(vec![Tag::ByteArray(vec![1]), Tag::ByteArray(vec![])]),
        Tag::List(vec![Tag::list([1i32]), Tag::list([2i32, 3])]),
        Tag::List(vec![Tag::Compound(compound! {"a" => 1i32}), Tag::Compound(Compound::new())]),
        Tag::List(vec![Tag::IntArray(vec![1, 2]), Tag::IntArray(vec![])]),
        Tag::List(vec![Tag::LongArray(vec![1, 2])]),
        Tag::Compound(Compound::new()),
        Tag::Compound(compound! {
            "b" => 1i8, "s" => 2i16, "i" => 3i32, "l" => 4i64, "f" => 5.5f32, "d" => 6.5f64,
            "ba" => Tag::ByteArray(vec![1, 2]), "str" => "text", "list" => Tag::list([1i32, 2]),
            "nested" => compound! {"deep" => compound! {"x" => 1i8}},
            "ia" => vec![1i32, 2, 3], "la" => vec![1i64, 2],
        }),
        Tag::IntArray(vec![]),
        Tag::IntArray(vec![i32::MIN, -1, 0, 1, i32::MAX]),
        Tag::LongArray(vec![]),
        Tag::LongArray(vec![i64::MIN, -1, 0, 1, i64::MAX]),
    ]
}

#[test]
fn round_trip_every_type_both_flavors() {
    for flavor in FLAVORS {
        for tag in every_type() {
            for name in ["", "hello", "caf\u{e9}"] {
                let bytes = io::to_vec(name, &tag, flavor).unwrap();
                let (n, back) = io::from_slice(&bytes, flavor).unwrap();
                assert_eq!((n.as_str(), &back), (name, &tag), "{flavor:?} {tag}");
                // Through the stream API as well.
                let (n2, back2) = io::read_named(&mut bytes.as_slice(), flavor).unwrap();
                assert_eq!((n2, back2), (n, back));
                let mut sink = Vec::new();
                io::write_named(&mut sink, name, &tag, flavor).unwrap();
                assert_eq!(sink, bytes);
            }
        }
    }
}

#[test]
fn nan_round_trips_bitwise() {
    for flavor in FLAVORS {
        let bytes = io::to_vec("", &Tag::Float(f32::NAN), flavor).unwrap();
        assert!(io::from_slice(&bytes, flavor).unwrap().1.as_f32().unwrap().is_nan());
        let bytes = io::to_vec("", &Tag::Double(f64::NAN), flavor).unwrap();
        assert!(io::from_slice(&bytes, flavor).unwrap().1.as_f64().unwrap().is_nan());
    }
}

#[test]
fn golden_java_bytes() {
    // The `hello world` example of the NBT specification, plus a few more entries.
    let tag = Tag::Compound(compound! { "name" => "Bananrama" });
    let expected: Vec<u8> =
        [&[0x0A, 0x00, 0x0B][..], b"hello world", &[0x08, 0x00, 0x04], b"name", &[0x00, 0x09], b"Bananrama", &[0x00]]
            .concat();
    assert_eq!(io::to_vec("hello world", &tag, Flavor::Java).unwrap(), expected);
    let (name, back) = io::from_slice(&expected, Flavor::Java).unwrap();
    assert_eq!(name, "hello world");
    assert_eq!(back, tag);
}

#[test]
fn golden_java_numbers_and_lists() {
    let tag = Tag::Compound(
        Compound::new()
            .with("b", -2i8)
            .with("s", 0x0102i16)
            .with("i", 0x01020304i32)
            .with("l", 0x0102030405060708i64)
            .with("f", 1.0f32)
            .with("d", 1.0f64)
            .with("li", Tag::list([7i32, 8]))
            .with("e", Tag::empty_list())
            .with("ia", vec![1i32])
            .with("la", vec![2i64])
            .with("ba", Tag::ByteArray(vec![-1, 5])),
    );
    let mut e: Vec<u8> = vec![0x0A, 0, 0];
    e.extend([1, 0, 1, b'b', 0xFE]);
    e.extend([2, 0, 1, b's', 1, 2]);
    e.extend([3, 0, 1, b'i', 1, 2, 3, 4]);
    e.extend([4, 0, 1, b'l', 1, 2, 3, 4, 5, 6, 7, 8]);
    e.extend([5, 0, 1, b'f', 0x3F, 0x80, 0, 0]);
    e.extend([6, 0, 1, b'd', 0x3F, 0xF0, 0, 0, 0, 0, 0, 0]);
    e.extend([9, 0, 2, b'l', b'i', 3, 0, 0, 0, 2, 0, 0, 0, 7, 0, 0, 0, 8]);
    e.extend([9, 0, 1, b'e', 0, 0, 0, 0, 0]);
    e.extend([11, 0, 2, b'i', b'a', 0, 0, 0, 1, 0, 0, 0, 1]);
    e.extend([12, 0, 2, b'l', b'a', 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2]);
    e.extend([7, 0, 2, b'b', b'a', 0, 0, 0, 2, 0xFF, 5]);
    e.push(0);
    assert_eq!(io::to_vec("", &tag, Flavor::Java).unwrap(), e);
    assert_eq!(io::from_slice(&e, Flavor::Java).unwrap().1, tag);
}

#[test]
fn golden_bedrock_bytes() {
    let tag = Tag::Compound(
        Compound::new().with("n", 0x01020304i32).with("s", "hi").with("l", Tag::list([1i16])).with("f", 1.0f32),
    );
    let mut e: Vec<u8> = vec![0x0A, 0, 0];
    e.extend([3, 1, 0, b'n', 4, 3, 2, 1]);
    e.extend([8, 1, 0, b's', 2, 0, b'h', b'i']);
    e.extend([9, 1, 0, b'l', 2, 1, 0, 0, 0, 1, 0]);
    e.extend([5, 1, 0, b'f', 0, 0, 0x80, 0x3F]);
    e.push(0);
    assert_eq!(io::to_vec("", &tag, Flavor::Bedrock).unwrap(), e);
    assert_eq!(io::from_slice(&e, Flavor::Bedrock).unwrap().1, tag);
}

#[test]
fn java_uses_modified_utf8_and_bedrock_plain() {
    let s = Tag::String("a\0\u{1F60E}".into());
    let java = io::to_vec("", &s, Flavor::Java).unwrap();
    // id, name len 0, string len 9: 'a', C0 80, then the emoji as two 3-byte surrogates.
    assert_eq!(java, [8, 0, 0, 0, 9, b'a', 0xC0, 0x80, 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x8E]);
    assert_eq!(io::from_slice(&java, Flavor::Java).unwrap().1, s);
    let bedrock = io::to_vec("", &s, Flavor::Bedrock).unwrap();
    assert_eq!(bedrock, [8, 0, 0, 6, 0, b'a', 0, 0xF0, 0x9F, 0x98, 0x8E]);
    assert_ne!(java, bedrock);
    // Standard UTF-8 (with a real 4-byte emoji) is accepted on the Java side, as some writers emit it.
    let std_utf8 = [8, 0, 0, 0, 4, 0xF0, 0x9F, 0x98, 0x8E];
    assert_eq!(io::from_slice(&std_utf8, Flavor::Java).unwrap().1, Tag::String("\u{1F60E}".into()));
}

#[test]
fn list_homogeneity() {
    assert!(Tag::try_list(vec![Tag::Int(1), Tag::Int(2)]).is_ok());
    assert!(matches!(Tag::try_list(vec![Tag::Int(1), Tag::Short(2)]), Err(NbtError::MixedList { .. })));
    let mixed = Tag::List(vec![Tag::Int(1), Tag::String("x".into())]);
    assert!(mixed.validate().is_err());
    for flavor in FLAVORS {
        assert!(matches!(io::to_vec("", &mixed, flavor), Err(NbtError::MixedList { .. })));
        let nested = Tag::Compound(compound! { "l" => mixed.clone() });
        assert!(io::to_vec("", &nested, flavor).is_err());
    }
    assert_eq!(Tag::list([1i32]).list_type(), Some(TagType::Int));
    assert_eq!(Tag::empty_list().list_type(), Some(TagType::End));
    assert_eq!(Tag::Int(1).list_type(), None);
}

#[test]
fn empty_and_end_typed_lists_normalize() {
    // An END-typed list is empty whatever its length says; a BYTE list of length 0 is empty as well.
    let end_with_len = [9, 0, 0, 0, 0, 0, 0, 5];
    assert_eq!(io::from_slice(&end_with_len, Flavor::Java).unwrap().1, Tag::List(vec![]));
    let byte_empty = [9, 0, 0, 1, 0, 0, 0, 0];
    assert_eq!(io::from_slice(&byte_empty, Flavor::Java).unwrap().1, Tag::List(vec![]));
}

#[test]
fn compound_keeps_insertion_order() {
    let mut c = Compound::new();
    for k in ["z", "a", "m", "b"] {
        c.put(k, 1i32);
    }
    c.put("a", 2i32); // replacing keeps the position
    assert_eq!(c.keys().map(String::as_str).collect::<Vec<_>>(), ["z", "a", "m", "b"]);
    c.remove("a");
    assert_eq!(c.keys().map(String::as_str).collect::<Vec<_>>(), ["z", "m", "b"]);
    for flavor in FLAVORS {
        let bytes = io::to_vec("", &Tag::Compound(c.clone()), flavor).unwrap();
        let back = io::from_slice(&bytes, flavor).unwrap().1;
        let keys: Vec<_> = back.as_compound().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["z", "m", "b"]);
    }
    // Equality ignores the order.
    let other = compound! {"b" => 1i32, "m" => 1i32, "z" => 1i32};
    assert_eq!(c, other);
}

#[test]
fn duplicate_names_later_wins() {
    let bytes = [10, 0, 0, 1, 0, 1, b'a', 1, 1, 0, 1, b'a', 2, 0];
    let c = io::from_slice(&bytes, Flavor::Java).unwrap().1;
    assert_eq!(c.as_compound().unwrap().len(), 1);
    assert_eq!(c.as_compound().unwrap().get_i8("a"), Some(2));
}

#[test]
fn compound_stream_reads_back_to_back_roots() {
    let a = Tag::Compound(compound! {"a" => 1i32});
    let b = Tag::Compound(compound! {"name" => "b"});
    let mut bytes = io::to_vec("", &a, Flavor::Bedrock).unwrap();
    bytes.extend(io::to_vec("x", &b, Flavor::Bedrock).unwrap());
    let roots = io::read_compound_stream(&bytes, Flavor::Bedrock).unwrap();
    assert_eq!(roots, vec![("".to_string(), a.clone()), ("x".to_string(), b)]);
    assert!(io::read_compound_stream(&[], Flavor::Bedrock).unwrap().is_empty());
    // A trailing END stops the stream; a trailing partial root is an error.
    let mut with_end = io::to_vec("", &a, Flavor::Bedrock).unwrap();
    with_end.push(0);
    assert_eq!(io::read_compound_stream(&with_end, Flavor::Bedrock).unwrap().len(), 1);
    let mut partial = io::to_vec("", &a, Flavor::Bedrock).unwrap();
    partial.extend([10, 0]);
    assert!(io::read_compound_stream(&partial, Flavor::Bedrock).is_err());
    // read_named leaves the rest of a stream alone.
    let mut cursor = bytes.as_slice();
    io::read_named(&mut cursor, Flavor::Bedrock).unwrap();
    assert_eq!(io::read_named(&mut cursor, Flavor::Bedrock).unwrap().0, "x");
    assert!(cursor.is_empty());
}

#[test]
fn end_root_is_empty_root() {
    for flavor in FLAVORS {
        assert!(matches!(io::from_slice(&[0], flavor), Err(NbtError::EmptyRoot)));
        assert!(matches!(io::from_slice(&[], flavor), Err(NbtError::UnexpectedEof)));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Hostile input

fn sample_doc(flavor: Flavor) -> Vec<u8> {
    let tag = Tag::Compound(compound! {
        "a" => 1i32, "name" => "caf\u{e9}", "list" => Tag::list([compound! {"x" => 1i8}, compound! {"y" => 2i16}]),
        "arrays" => compound! {"i" => vec![1i32, 2, 3], "l" => vec![4i64], "b" => Tag::ByteArray(vec![1, 2, 3])},
        "f" => 1.5f32, "d" => 2.5f64, "long" => 5i64,
    });
    io::to_vec("root", &tag, flavor).unwrap()
}

#[test]
fn every_truncation_is_an_error() {
    for flavor in FLAVORS {
        let doc = sample_doc(flavor);
        assert!(io::from_slice(&doc, flavor).is_ok());
        for cut in 0..doc.len() {
            assert!(io::from_slice(&doc[..cut], flavor).is_err(), "{flavor:?} cut at {cut}");
            assert!(io::read_named(&mut &doc[..cut], flavor).is_err());
        }
    }
}

#[test]
fn random_corruption_never_panics() {
    let mut x = 0x2545_F491u32;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for flavor in FLAVORS {
        let doc = sample_doc(flavor);
        for _ in 0..20_000 {
            let mut d = doc.clone();
            for _ in 0..1 + next() % 4 {
                let i = next() as usize % d.len();
                d[i] = next() as u8;
            }
            let _ = io::from_slice(&d, flavor);
            let _ = io::read_compound_stream(&d, flavor);
            let _ = io::read_named(&mut d.as_slice(), flavor);
        }
    }
}

#[test]
fn huge_lengths_are_rejected_without_allocating() {
    for flavor in FLAVORS {
        let be = flavor == Flavor::Java;
        let int = |v: i32| if be { v.to_be_bytes() } else { v.to_le_bytes() };
        for (id, len) in [(7u8, i32::MAX), (11, i32::MAX), (12, i32::MAX), (7, -1), (11, MAX_ARRAY_LENGTH as i32 + 1)] {
            let mut d = vec![id, 0, 0];
            d.extend(int(len));
            assert!(matches!(io::from_slice(&d, flavor), Err(NbtError::LimitExceeded { .. })), "id {id} len {len}");
        }
        // A list claiming the maximum with no data behind it, and one above the maximum.
        for len in [MAX_LIST_LENGTH as i32, MAX_LIST_LENGTH as i32 + 1, i32::MAX, -5] {
            let mut d = vec![9, 0, 0, 3];
            d.extend(int(len));
            assert!(io::from_slice(&d, flavor).is_err(), "list {len}");
            assert!(io::read_named(&mut d.as_slice(), flavor).is_err());
        }
        // A maximal array claim with a short body: error, through both reader kinds.
        let mut d = vec![7, 0, 0];
        d.extend(int(MAX_ARRAY_LENGTH as i32));
        d.extend([1, 2, 3]);
        assert!(matches!(io::from_slice(&d, flavor), Err(NbtError::UnexpectedEof)));
        assert!(matches!(io::read_named(&mut d.as_slice(), flavor), Err(NbtError::UnexpectedEof)));
    }
}

#[test]
fn string_limits() {
    // Bedrock: negative or over-long prefixes.
    assert!(matches!(io::from_slice(&[8, 0, 0, 0xFF, 0xFF], Flavor::Bedrock), Err(NbtError::LimitExceeded { .. })));
    let mut d = vec![8, 0, 0];
    d.extend((io::MAX_STRING_LENGTH as u16 + 1).to_le_bytes());
    d.extend(vec![b'x'; io::MAX_STRING_LENGTH + 1]);
    assert!(matches!(io::from_slice(&d, Flavor::Bedrock), Err(NbtError::LimitExceeded { .. })));
    // The name limit is 256.
    for flavor in FLAVORS {
        let ok = "n".repeat(256);
        let bytes = io::to_vec(&ok, &Tag::Byte(1), flavor).unwrap();
        assert_eq!(io::from_slice(&bytes, flavor).unwrap().0, ok);
        let bytes = io::to_vec(&"n".repeat(257), &Tag::Byte(1), flavor).unwrap();
        assert!(matches!(io::from_slice(&bytes, flavor), Err(NbtError::LimitExceeded { .. })));
    }
    // The longest string value reads back; the writer refuses what the reader would not accept (Bedrock).
    let long = Tag::String("x".repeat(io::MAX_STRING_LENGTH));
    for flavor in FLAVORS {
        let bytes = io::to_vec("", &long, flavor).unwrap();
        assert_eq!(io::from_slice(&bytes, flavor).unwrap().1, long);
    }
    assert!(io::to_vec("", &Tag::String("x".repeat(io::MAX_STRING_LENGTH + 1)), Flavor::Bedrock).is_err());
    assert!(io::to_vec("", &Tag::String("x".repeat(70_000)), Flavor::Java).is_err());
    // Java counts UTF-16 units of the decoded string, not bytes: 20000 two-byte characters are fine.
    let wide = Tag::String("\u{e9}".repeat(20_000));
    let bytes = io::to_vec("", &wide, Flavor::Java).unwrap();
    assert_eq!(io::from_slice(&bytes, Flavor::Java).unwrap().1, wide);
}

#[test]
fn bad_utf8_is_an_error() {
    // Bedrock: 0xC3 0x28 is not UTF-8.
    assert!(matches!(io::from_slice(&[8, 0, 0, 2, 0, 0xC3, 0x28], Flavor::Bedrock), Err(NbtError::BadString(_))));
    assert!(io::from_slice(&[8, 0, 0, 1, 0, 0xFF], Flavor::Bedrock).is_err());
    // As a name too.
    assert!(io::from_slice(&[1, 1, 0, 0xFF, 0], Flavor::Bedrock).is_err());
    // Java: truncated sequence, bad continuation, invalid lead, lone surrogate.
    for body in [&[0xE2, 0x82][..], &[0xC3, 0x28], &[0xF8, 0x80], &[0xED, 0xA0, 0xBD], &[0xC3]] {
        let mut d = vec![8, 0, 0, 0, body.len() as u8];
        d.extend(body);
        assert!(matches!(io::from_slice(&d, Flavor::Java), Err(NbtError::BadString(_))), "{body:x?}");
    }
}

#[test]
fn unknown_tag_ids() {
    for id in [13u8, 14, 100, 255] {
        for flavor in FLAVORS {
            assert!(matches!(io::from_slice(&[id, 0, 0], flavor), Err(NbtError::UnknownTagId(i)) if i == id));
            // Inside a compound and as a list element type.
            assert!(io::from_slice(&[10, 0, 0, id, 0, 0, 0], flavor).is_err());
            assert!(io::from_slice(&[10, 0, 0, 9, 0, 1, b'l', id, 0, 0, 0, 1, 0], flavor).is_err());
        }
    }
}

#[test]
fn deep_nesting_is_an_error() {
    let n = 100_000;
    for flavor in FLAVORS {
        // Compounds nested n deep: each level is `0A 00 00` (an unnamed compound entry is id + empty name).
        let mut d = vec![10, 0, 0];
        for _ in 0..n {
            d.extend([10, 0, 0]);
        }
        assert!(matches!(io::from_slice(&d, flavor), Err(NbtError::TooDeep)));
        // Lists of lists: `09 00 00 | 09 00 00 00 01 | ...`.
        let be = flavor == Flavor::Java;
        let one = if be { 1i32.to_be_bytes() } else { 1i32.to_le_bytes() };
        let mut d = vec![9, 0, 0];
        for _ in 0..n {
            d.push(9);
            d.extend(one);
        }
        assert!(matches!(io::from_slice(&d, flavor), Err(NbtError::TooDeep)));
        // The deepest allowed nesting reads and writes; one more does not.
        let mut deepest = Tag::Compound(Compound::new());
        for _ in 0..MAX_DEPTH - 1 {
            deepest = Tag::Compound(compound! {"c" => deepest});
        }
        let bytes = io::to_vec("", &deepest, flavor).unwrap();
        assert_eq!(io::from_slice(&bytes, flavor).unwrap().1, deepest);
        let too_deep = Tag::Compound(compound! {"c" => deepest.clone()});
        assert!(matches!(io::to_vec("", &too_deep, flavor), Err(NbtError::TooDeep)));
        // Dropping a deep tree must not overflow the stack either (nesting is capped at the limit).
        drop(deepest);
    }
}

#[test]
fn too_many_compound_entries() {
    // MAX_COMPOUND_LENGTH + 1 distinct byte entries.
    let n = io::MAX_COMPOUND_LENGTH + 1;
    let mut d = vec![10, 0, 0];
    for i in 0..n {
        let name = format!("{i:07}");
        d.extend([1, 0, name.len() as u8]);
        d.extend(name.as_bytes());
        d.push(0);
    }
    d.push(0);
    assert!(matches!(io::from_slice(&d, Flavor::Java), Err(NbtError::LimitExceeded { what: "compound", .. })));
}

#[test]
fn decompression_bombs_and_garbage() {
    assert!(io::gunzip(&[1, 2, 3]).is_err());
    assert!(io::unzlib(&[1, 2, 3]).is_err());
    assert!(file::decompress(&[0; 30], Compression::Lz4Block).is_err());
    assert!(file::read_java_nbt(&[0x1F, 0x8B, 8, 0], Compression::Gzip).is_err());
    // A truncated gzip stream.
    let gz = io::gzip(&sample_doc(Flavor::Java)).unwrap();
    assert!(io::gunzip(&gz[..gz.len() / 2]).is_err());
}

// ---------------------------------------------------------------------------------------------------------------
// Compound API (ports of Chunker's CompoundTagTests)

fn sample() -> Compound {
    compound! {
        "byte" => 1i8, "short" => 2i16, "int" => 3i32, "long" => 4i64, "float" => 5.5f32, "double" => 6.5f64,
        "string" => "text", "bytes" => Tag::ByteArray(vec![1, 2]), "ints" => vec![1i32, 2], "longs" => vec![3i64],
        "compound" => compound! {"inner" => 1i32}, "list" => Tag::list([1i32, 2]), "strings" => Tag::list(["a", "b"]),
        "flag" => true, "empty_list" => Tag::empty_list(), "no_list" => Tag::LongArray(vec![]),
    }
}

#[test]
fn typed_getters() {
    let c = sample();
    assert_eq!(c.get_i8("byte"), Some(1));
    assert_eq!(c.get_i16("short"), Some(2));
    assert_eq!(c.get_i32("int"), Some(3));
    assert_eq!(c.get_i64("long"), Some(4));
    assert_eq!(c.get_f32("float"), Some(5.5));
    assert_eq!(c.get_f64("double"), Some(6.5));
    assert_eq!(c.get_str("string"), Some("text"));
    assert_eq!(c.get_byte_array("bytes"), Some(&[1i8, 2][..]));
    assert_eq!(c.get_int_array("ints"), Some(&[1, 2][..]));
    assert_eq!(c.get_long_array("longs"), Some(&[3i64][..]));
    assert_eq!(c.get_compound("compound").unwrap().get_i32("inner"), Some(1));
    assert_eq!(c.get_list("list").unwrap().len(), 2);
    assert_eq!(c.get_bool("flag"), Some(true));
    assert_eq!(c.get_bool("byte"), Some(true));
    // Missing and wrong-typed keys are None.
    assert_eq!(c.get_i32("missing"), None);
    assert_eq!(c.get_i32("short"), None);
    assert_eq!(c.get_str("int"), None);
    assert_eq!(c.get_compound("string"), None);
    assert_eq!(c.get_integer("short"), Some(2));
    assert_eq!(c.get_number("float"), Some(5.5));
}

#[test]
fn defaults_and_fallbacks() {
    let c = sample();
    assert_eq!(c.get_i32_or("int", 9), 3);
    assert_eq!(c.get_i32_or("missing", 9), 9);
    assert_eq!(c.get_i32_or("short", 9), 9);
    assert_eq!(c.get_str_or("missing", "dflt"), "dflt");
    assert!(c.get_bool_or("missing", true));
    let empty = Compound::new();
    assert_eq!(c.get_compound_or("missing", &empty).len(), 0);
}

#[test]
fn require_errors_on_missing_and_wrong_type() {
    let c = sample();
    assert_eq!(c.require_i32("int").unwrap(), 3);
    assert_eq!(c.require_str("string").unwrap(), "text");
    assert_eq!(c.require_i8("byte").unwrap(), 1);
    assert!(matches!(c.require_i32("missing"), Err(NbtError::Missing(k)) if k == "missing"));
    match c.require_i32("short") {
        Err(NbtError::WrongType { key, found, .. }) => {
            assert_eq!(key, "short");
            assert_eq!(found, TagType::Short);
        }
        other => panic!("{other:?}"),
    }
    assert!(c.require_compound("string").is_err());
    assert!(c.require_bool("string").is_err());
    assert!(c.require_bool("flag").unwrap());
    // Chunker's `getX(name, default)`: absent is fine, a wrong type still fails.
    assert_eq!(c.opt_i32("missing").unwrap(), None);
    assert_eq!(c.opt_i32("int").unwrap(), Some(3));
    assert!(c.opt_i32("short").is_err());
    assert!(c.require_i32("missing").unwrap_err().to_string().contains("Could not access missing"));
}

#[test]
fn list_getters_follow_chunker() {
    let c = sample();
    assert_eq!(c.require_list_of("list", TagType::Int).unwrap().len(), 2);
    assert!(c.require_list_of("list", TagType::String).is_err());
    assert!(c.require_list_of("missing", TagType::Int).is_err());
    // Empty lists match any element type.
    assert!(c.require_list_of("empty_list", TagType::Compound).unwrap().is_empty());
    // Absent and "empty long array" both read as no list.
    assert_eq!(c.opt_list_of("missing", TagType::Int).unwrap(), None);
    assert_eq!(c.opt_list_of("no_list", TagType::Int).unwrap(), None);
    // A non-empty long array is an error, so is any other tag.
    assert!(c.opt_list_of("longs", TagType::Int).is_err());
    assert!(c.opt_list_of("string", TagType::Int).is_err());
    assert_eq!(c.require_list_values::<i32>("list").unwrap(), vec![1, 2]);
    assert_eq!(c.require_list_values::<String>("strings").unwrap(), vec!["a".to_string(), "b".to_string()]);
    assert!(c.require_list_values::<i64>("list").is_err());
    assert_eq!(c.opt_list_values::<i32>("missing").unwrap(), None);
    assert_eq!(<i32 as FromTag>::TYPE, TagType::Int);
}

#[test]
fn mutation_helpers() {
    let mut c = Compound::new();
    c.get_or_create_compound("a").put("x", 1i32);
    c.get_or_create_compound("a").put("y", 2i32);
    assert_eq!(c.get_compound("a").unwrap().len(), 2);
    c.put("a", "now a string");
    assert_eq!(c.get_or_create_compound("a").len(), 0);
    assert_eq!(c.put("k", 1i8), None);
    assert_eq!(c.put("k", 2i8), Some(Tag::Byte(1)));
    assert_eq!(c.remove("k"), Some(Tag::Byte(2)));
    assert_eq!(c.remove("k"), None);
    assert!(!c.contains_key("k"));
    c.get_compound_mut("a").unwrap().put("z", 1i32);
    assert_eq!(c.get_compound("a").unwrap().get_i32("z"), Some(1));
    c.get_or_create_compound("l");
    c.put("list", Tag::list([1i32]));
    c.get_list_mut("list").unwrap().push(Tag::Int(2));
    assert_eq!(c.get_list("list").unwrap().len(), 2);
    let built: Compound = [("a", 1i32), ("b", 2)].into_iter().collect();
    assert_eq!(built.len(), 2);
    assert_eq!(built.into_iter().map(|(k, _)| k).collect::<Vec<_>>(), ["a", "b"]);
}

#[test]
fn from_impls_and_tag_helpers() {
    assert_eq!(Tag::from(true), Tag::Byte(1));
    assert_eq!(Tag::from(false), Tag::Byte(0));
    assert_eq!(Tag::from(200u8), Tag::Byte(-56));
    assert_eq!(Tag::from(vec![255u8, 1]), Tag::ByteArray(vec![-1, 1]));
    assert_eq!(Tag::from("x"), Tag::String("x".into()));
    assert_eq!(Tag::from(&[1i32, 2][..]), Tag::IntArray(vec![1, 2]));
    for (tag, id) in every_type().into_iter().map(|t| (t.clone(), t.type_id())) {
        assert_eq!(TagType::from_id(id), Some(tag.tag_type()));
        assert_eq!(tag.tag_type().id(), id);
    }
    assert_eq!(TagType::from_id(13), None);
    assert_eq!(TagType::ALL.len(), 13);
    assert_eq!(Tag::Short(5).as_integer(), Some(5));
    assert_eq!(Tag::Int(5).as_i16(), None);
    assert_eq!(Tag::Float(1.5).as_number(), Some(1.5));
    assert_eq!(Tag::String("s".into()).as_integer(), None);
    assert_eq!(Tag::Byte(2).as_bool(), Some(true));
    let mut t = Tag::String("a".into());
    t.as_string_mut().unwrap().push('b');
    assert_eq!(t.as_str(), Some("ab"));
    let cloned = t.clone();
    assert_eq!(t, cloned);
    assert_ne!(Tag::Byte(1), Tag::Int(1));
}

#[test]
fn snbt_text() {
    let t = Tag::Compound(
        Compound::new()
            .with("b", 1i8)
            .with("s", 2i16)
            .with("i", 3i32)
            .with("l", 4i64)
            .with("f", 1.0f32)
            .with("d", 2.5f64)
            .with("str", "q\"uote\n")
            .with("ba", Tag::ByteArray(vec![1, -2]))
            .with("ia", vec![1i32, 2])
            .with("la", vec![3i64])
            .with("list", Tag::list([1i32, 2]))
            .with("a\"b", 0i8)
            .with("c", Compound::new()),
    );
    assert_eq!(
        t.to_snbt(),
        "{b:1b,s:2s,i:3,l:4l,f:1.0f,d:2.5,str:\"q\\\"uote\\u000A\",ba:[B;1b,-2b],ia:[I;1,2],la:[L;3l],list:[1,2],\"a\\\"b\":0b,c:{}}"
    );
    assert_eq!(Tag::Double(1e10).to_snbt(), "1.0E10");
    assert_eq!(Tag::Double(1.5e-5).to_snbt(), "1.5E-5");
    assert_eq!(Tag::Double(0.001).to_snbt(), "0.001");
    assert_eq!(Tag::Double(-0.0).to_snbt(), "-0.0");
    assert_eq!(Tag::Double(f64::NAN).to_snbt(), "NaN");
    assert_eq!(Tag::Float(f32::INFINITY).to_snbt(), "Infinityf");
    assert_eq!(Tag::empty_list().to_string(), "[]");
}

// ---------------------------------------------------------------------------------------------------------------
// Files and compression

fn level_like() -> Compound {
    compound! { "Data" => compound! { "LevelName" => "World", "allowCommands" => 0i8, "DataVersion" => 3955i32 } }
}

#[test]
fn java_file_wrappers_round_trip() {
    for c in [Compression::None, Compression::Gzip, Compression::Zlib, Compression::Lz4Block] {
        let bytes = file::write_java_nbt(&level_like(), c).unwrap();
        // Reading unwraps `Data`.
        let read = file::read_java_nbt(&bytes, c).unwrap().unwrap();
        assert_eq!(read.get_str("LevelName"), Some("World"), "{c:?}");
        // The root reader does not.
        assert_eq!(file::read_java_root(&bytes, c).unwrap(), level_like());
        assert_eq!(Compression::from_anvil_id(c.anvil_id()), Some(c));
    }
    assert_eq!(Compression::from_anvil_id(9), None);
    // Lowercase `data` is unwrapped too; no wrapper leaves the root alone.
    let lower = compound! { "data" => compound! { "x" => 1i32 } };
    let bytes = file::write_java_nbt(&lower, Compression::None).unwrap();
    assert_eq!(file::read_java_nbt(&bytes, Compression::None).unwrap().unwrap().get_i32("x"), Some(1));
    let plain = compound! { "x" => 1i32 };
    let bytes = file::write_java_nbt(&plain, Compression::Zlib).unwrap();
    assert_eq!(file::read_java_nbt(&bytes, Compression::Zlib).unwrap(), Some(plain));
    // An END root reads as nothing, a non-compound root is an error.
    assert_eq!(file::read_java_nbt(&[0], Compression::None).unwrap(), None);
    let list_root = io::to_vec("", &Tag::empty_list(), Flavor::Java).unwrap();
    assert!(file::read_java_nbt(&list_root, Compression::None).is_err());
}

#[test]
fn compression_helpers_and_auto_detection() {
    let raw = io::to_vec("", &Tag::Compound(level_like()), Flavor::Java).unwrap();
    assert_eq!(io::gunzip(&io::gzip(&raw).unwrap()).unwrap(), raw);
    assert_eq!(io::unzlib(&io::zlib(&raw).unwrap()).unwrap(), raw);
    assert_eq!(io::decompress_auto(&io::gzip(&raw).unwrap()).unwrap(), raw);
    assert_eq!(io::decompress_auto(&io::zlib(&raw).unwrap()).unwrap(), raw);
    assert_eq!(io::decompress_auto(&raw).unwrap(), raw);
    assert_eq!(io::decompress_auto(&[]).unwrap(), Vec::<u8>::new());
    // Bedrock plain NBT too (starts with 0A 00 00).
    let bedrock = io::to_vec("", &Tag::Compound(level_like()), Flavor::Bedrock).unwrap();
    assert_eq!(io::decompress_auto(&bedrock).unwrap(), bedrock);
    // Raw deflate (LevelDB blocks).
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut enc, &raw.repeat(5)).unwrap();
    let deflated = enc.finish().unwrap();
    assert_eq!(io::decompress_auto(&deflated).unwrap(), raw.repeat(5));
    // The first byte of the zlib stream Chunker writes is 0x78 (best speed: 78 01).
    assert_eq!(&io::zlib(&raw).unwrap()[..2], &[0x78, 0x01]);
}

#[test]
fn bedrock_level_dat_header() {
    let root = compound! { "LevelName" => "Bedrock world", "StorageVersion" => 10i32, "spawnX" => -3i32 };
    let bytes = file::write_bedrock_level_dat(10, &root).unwrap();
    assert_eq!(&bytes[..4], &10i32.to_le_bytes());
    assert_eq!(&bytes[4..8], &((bytes.len() - 8) as i32).to_le_bytes());
    let (version, back) = file::read_bedrock_level_dat(&bytes).unwrap();
    assert_eq!(version, 10);
    assert_eq!(back, Some(root.clone()));
    assert!(file::read_bedrock_level_dat(&bytes[..5]).is_err());
    assert_eq!(file::read_bedrock_nbt(&[]).unwrap(), None);
    assert_eq!(file::read_bedrock_nbt(&file::write_bedrock_nbt(&root).unwrap()).unwrap(), Some(root));
}

#[test]
fn file_io() {
    let dir = std::env::temp_dir().join(format!("bf-nbt-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("level.dat");
    file::write_java_file(&path, &level_like(), Compression::Gzip).unwrap();
    assert_eq!(file::read_possible_gzip_java_file(&path).unwrap().unwrap().get_i32("DataVersion"), Some(3955));
    assert_eq!(file::read_java_file(&path, Compression::Gzip).unwrap().unwrap().get_str("LevelName"), Some("World"));
    file::write_java_file(&path, &level_like(), Compression::None).unwrap();
    assert!(file::read_possible_gzip_java_file(&path).unwrap().is_some());
    assert!(file::read_java_file(dir.join("missing.dat"), Compression::None).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bedrock_test_world_level_dat() {
    use std::io::Read;
    let path = std::path::Path::new("/home/riyadm/Downloads/Mace, Spear, Sword PvP Map! V4.mcworld");
    let Ok(f) = std::fs::File::open(path) else {
        eprintln!("skipped: {} not found", path.display());
        return;
    };
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut bytes = Vec::new();
    zip.by_name("level.dat").unwrap().read_to_end(&mut bytes).unwrap();
    let (version, root) = file::read_bedrock_level_dat(&bytes).unwrap();
    assert!((5..=100).contains(&version), "storage version {version}");
    assert_eq!(i32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len() - 8);
    let root = root.unwrap();
    assert!(root.get_str("LevelName").is_some_and(|n| !n.is_empty()));
    assert!(root.get_i32("StorageVersion").is_some());
    assert!(root.get_i32("GameType").is_some());
    assert!(root.get_compound("abilities").is_some() || root.get_i32("NetworkVersion").is_some());
    // Re-encoding gives back the same payload bytes when decoded again (order and types preserved).
    let again = file::write_bedrock_nbt(&root).unwrap();
    assert_eq!(file::read_bedrock_nbt(&again).unwrap(), Some(root.clone()));
    assert_eq!(&again[..], &bytes[8..], "byte-exact re-encode of level.dat");
}

// ---------------------------------------------------------------------------------------------------------------
// Performance

fn chunk_like() -> Tag {
    let mut sections = Vec::new();
    for y in -4..20i8 {
        let palette: Vec<Tag> = (0..12)
            .map(|i| {
                Tag::Compound(
                    Compound::new()
                        .with("Name", format!("minecraft:block_{i}"))
                        .with("Properties", compound! {"facing" => "north", "waterlogged" => "false"}),
                )
            })
            .collect();
        sections.push(Tag::Compound(compound! {
            "Y" => y,
            "block_states" => compound! {
                "palette" => Tag::List(palette),
                "data" => (0..4096i64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64)).collect::<Vec<_>>(),
            },
            "biomes" => compound! {"palette" => Tag::list(["minecraft:plains"]), "data" => vec![0i64; 8]},
            "BlockLight" => Tag::ByteArray(vec![0; 2048]),
            "SkyLight" => Tag::ByteArray(vec![-1; 2048]),
        }));
    }
    Tag::Compound(compound! {
        "DataVersion" => 3955i32, "xPos" => 3i32, "zPos" => -7i32, "Status" => "minecraft:full",
        "sections" => Tag::List(sections),
        "Heightmaps" => compound! {"MOTION_BLOCKING" => vec![1i64; 37], "WORLD_SURFACE" => vec![2i64; 37]},
        "block_entities" => Tag::empty_list(),
    })
}

#[test]
fn reading_a_megabyte_chunk_is_fast() {
    for flavor in FLAVORS {
        let tag = chunk_like();
        let bytes = io::to_vec("", &tag, flavor).unwrap();
        assert!(bytes.len() > 700_000, "{} bytes", bytes.len());
        assert_eq!(io::from_slice(&bytes, flavor).unwrap().1, tag);
        let rounds = 20;
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(io::from_slice(std::hint::black_box(&bytes), flavor).unwrap());
        }
        let per = start.elapsed() / rounds;
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(io::to_vec("", std::hint::black_box(&tag), flavor).unwrap());
        }
        let per_write = start.elapsed() / rounds;
        eprintln!("{flavor:?}: {} bytes, read {per:?}, write {per_write:?}", bytes.len());
        // Generous: the unoptimized test profile is several times slower than release.
        assert!(per.as_millis() < 50, "read took {per:?}");
    }
}
