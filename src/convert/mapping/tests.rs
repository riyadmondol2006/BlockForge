use super::*;

fn s(x: &str) -> StateValue {
    StateValue::str(x)
}

#[test]
fn explore() {
    let r = BlockResolver::new(Edition::Java, Version::new(1, 21, 4));
    println!("{:?}", r.row_counts());
    let cases: Vec<(&str, StateMap)> = vec![
        ("minecraft:oak_stairs", states(&[("facing", s("north")), ("half", s("bottom")), ("shape", s("straight")), ("waterlogged", s("false"))])),
        ("minecraft:oak_stairs", states(&[("facing", s("east")), ("half", s("top")), ("shape", s("inner_left")), ("waterlogged", s("true"))])),
        ("minecraft:grass", states(&[])),
    ];
    for (id, st) in cases {
        let p = r.to_pivot(id, &st);
        println!("{id} -> {p:?}");
        if let Some(p) = p {
            println!("  back {:?}", r.from_pivot(&p));
        }
    }
}
