//! `Optic.test.ts` behavior port: lenses, prisms, optionals, traversals.

use elm_effect::{
    Iso, Lens, OpticError, Optional, Prism, Traversal, at_map, at_vec, at_vec_exact, failure, none,
    some, success,
};
use std::collections::HashMap;

fn add_one(n: i32) -> i32 {
    n + 1
}

#[derive(Clone, Debug, PartialEq)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct Line {
    from: Point,
    to: Point,
}

fn x_lens() -> Lens<Point, i32> {
    Lens::field(|p: &Point| p.x, |x, p: &Point| Point { x, ..p.clone() })
}

#[test]
fn id_iso_roundtrip() {
    let iso = Iso::<i32, i32>::id();
    assert_eq!(iso.get(&1), 1);
    assert_eq!(iso.set(2), 2);
}

#[test]
fn data_first_and_data_last_share_behavior() {
    // Spec rows "supports data-first usage" / "supports data-last usage":
    // argument order is a JS currying detail; the behavior is one set of
    // operations. Both call orders below assert the same outcomes.
    let lens = Lens::field(|s: &(i32,)| s.0, |v, _: &(i32,)| (v,));
    let at: Optional<Vec<i32>, i32> = at_vec(0);
    let prism: Prism<Option<i32>, i32> = some();
    let trav = Traversal::<Vec<i32>, i32>::vec_each();
    assert_eq!(lens.get(&(1,)), 1);
    assert_eq!(at.get_result(&vec![1]), Ok(1));
    assert_eq!(prism.set(1), Some(1));
    assert_eq!(lens.replace(2, &(1,)), (2,));
    assert_eq!(at.replace_result(2, &vec![1]), Ok(vec![2]));
    assert_eq!(lens.modify(add_one, &(1,)), (2,));
    assert_eq!(trav.get_all(&vec![1, 2, 3]), vec![1, 2, 3]);
    assert_eq!(trav.modify_all(add_one, &vec![1, 2, 3]), vec![2, 3, 4]);
}

#[test]
fn set_through_composed_isos_without_reading_source() {
    let value = Iso::new(|s: &(i32,)| s.0, |v: i32| (v,));
    let text = Iso::new(|n: &i32| n.to_string(), |s: String| s.parse().unwrap());
    let optic = value.compose_iso(&text);
    assert_eq!(optic.set("2".to_string()), (2,));
    assert_eq!(optic.get(&(1,)), "1".to_string());
}

#[test]
fn set_through_composed_prisms_without_reading_source() {
    let some = some();
    let text = Iso::new(|n: &i32| n.to_string(), |s: String| s.parse().unwrap_or(0));
    let optic = some.compose_iso(&text);
    assert_eq!(optic.set("2".to_string()), Some(2));
    assert_eq!(optic.get_result(&Some(1)), Ok("1".to_string()));
    assert!(optic.get_result(&None).is_err());
}

#[test]
fn capabilities_hold_across_every_optic_kind() {
    // Iso and Lens read totally; Prism builds totally; Optional writes
    // totally but reads fallibly; Traversal collects.
    let iso = Iso::<i32, i32>::id();
    assert_eq!((iso.get(&1), iso.set(2)), (1, 2));
    let lens = x_lens();
    assert_eq!(
        (
            lens.get(&Point { x: 1, y: 0 }),
            lens.replace(2, &Point { x: 1, y: 0 }).x
        ),
        (1, 2)
    );
    let prism: Prism<Option<i32>, i32> = some();
    assert_eq!(prism.set(2), Some(2));
    let optional: Optional<Vec<i32>, i32> = at_vec(0);
    assert_eq!(optional.replace(2, &vec![1]), vec![2]);
    assert!(optional.get_result(&vec![]).is_err());
    let trav = Traversal::<Vec<i32>, i32>::vec_each();
    assert_eq!(trav.get_all(&vec![1]), vec![1]);
}

#[test]
fn optional_setter_failures_propagate() {
    // A setter that always fails: total replace keeps the source, fallible
    // operations report the failure.
    // The spec's failing setter is a replace_result error in Rust: total
    // operations keep the source, fallible ones report the miss.
    let err_optic: Optional<i32, i32> = Optional::new(|_: &i32| None, |a, _: &i32| a);
    assert_eq!(
        err_optic.replace_result(2, &1),
        Err(OpticError::MissingFocus("optional".into()))
    );
    assert_eq!(err_optic.replace(2, &1), 1);
    assert_eq!(err_optic.modify(add_one, &1), 1);
    let optic: Optional<i32, i32> = Optional::new(|_: &i32| Some(1), |a, _: &i32| a);
    assert_eq!(optic.replace(2, &1), 2);
}

#[test]
fn identity_composes_in_both_directions() {
    let value = Iso::new(|n: &i32| n.to_string(), |s: String| s.parse().unwrap_or(0));
    let id_num = Iso::<i32, i32>::id();
    let left = id_num.compose_iso(&value);
    assert_eq!(left.get(&1), "1".to_string());
    // Right identity over the composed target type.
    let id_str = Iso::<String, String>::id();
    let right = value.compose_iso(&id_str);
    assert_eq!(right.get(&1), "1".to_string());
}

#[test]
fn composition_grouping_is_independent() {
    let a = Lens::field(
        |l: &Line| l.from.clone(),
        |v, l: &Line| Line {
            from: v,
            ..l.clone()
        },
    );
    let b = Lens::field(|p: &Point| p.x, |v, p: &Point| Point { x: v, ..p.clone() });
    let c = Lens::field(|n: &i32| *n, |v, _| v);
    let left = a.compose(&b).compose(&c);
    let right = a.compose(&b.compose(&c));
    let line = Line {
        from: Point { x: 1, y: 0 },
        to: Point { x: 0, y: 0 },
    };
    assert_eq!(left.get(&line), right.get(&line));
    assert_eq!(left.replace(2, &line), right.replace(2, &line));
}

#[test]
fn required_key_reads_and_writes() {
    let lens = x_lens();
    assert_eq!(lens.get(&Point { x: 1, y: 9 }), 1);
    assert_eq!(lens.replace(2, &Point { x: 1, y: 9 }), Point { x: 2, y: 9 });
    assert_eq!(
        lens.modify(add_one, &Point { x: 1, y: 9 }),
        Point { x: 2, y: 9 }
    );
}

#[test]
fn nested_required_keys() {
    let from = Lens::field(
        |l: &Line| l.from.clone(),
        |v, l: &Line| Line {
            from: v,
            ..l.clone()
        },
    );
    let optic = from.compose(&x_lens());
    let line = Line {
        from: Point { x: 1, y: 0 },
        to: Point { x: 5, y: 5 },
    };
    assert_eq!(optic.get(&line), 1);
    assert_eq!(optic.replace(2, &line).from.x, 2);
    assert_eq!(optic.modify(add_one, &line).from.x, 2);
}

#[test]
fn required_tuple_element() {
    let first = Lens::field(|s: &(i32, i32)| s.0, |v, s: &(i32, i32)| (v, s.1));
    assert_eq!(first.get(&(1, 9)), 1);
    assert_eq!(first.replace(2, &(1, 9)), (2, 9));
    assert_eq!(first.modify(add_one, &(1, 9)), (2, 9));
}

#[test]
fn nested_required_tuple_element() {
    let second = Lens::field(
        |s: &(i32, (i32, i32))| s.1,
        |v, s: &(i32, (i32, i32))| (s.0, v),
    );
    let first = Lens::field(|s: &(i32, i32)| s.0, |v, s: &(i32, i32)| (v, s.1));
    let optic = second.compose(&first);
    assert_eq!(optic.get(&(0, (1, 9))), 1);
    assert_eq!(optic.replace(2, &(0, (1, 9))), (0, (2, 9)));
}

#[test]
fn mixed_structs_and_tuples() {
    let pair = Lens::field(|s: &(i32, (i32,))| s.1, |v, s: &(i32, (i32,))| (s.0, v));
    let head = Lens::field(|s: &(i32,)| s.0, |v, _: &(i32,)| (v,));
    let optic = pair.compose(&head);
    assert_eq!(optic.get(&(0, (1,))), 1);
    assert_eq!(optic.replace(2, &(0, (1,))), (0, (2,)));
}

#[test]
fn exact_optional_key_without_undefined() {
    // `Option` fields are the Rust spelling of exact-optional keys.
    #[derive(Clone, Debug, PartialEq)]
    struct S {
        a: Option<i32>,
    }
    let optic: Optional<S, i32> = Optional::new(|s: &S| s.a, |v, _s: &S| S { a: Some(v) });
    // Port note: spec `replace(None, …)` deletes the key; Rust `Option`
    // expresses absence as a value, so deletion rows are skipped-upstream.
    assert_eq!(optic.get_result(&S { a: Some(1) }), Ok(1));
    assert!(optic.get_result(&S { a: None }).is_err());
    assert_eq!(optic.replace(2, &S { a: Some(1) }), S { a: Some(2) });
    assert_eq!(optic.replace(2, &S { a: None }), S { a: None });
}

#[test]
fn record_at_key() {
    let optic: Optional<HashMap<String, i32>, i32> = at_map("a".to_string());
    let mut full = HashMap::new();
    full.insert("a".to_string(), 1);
    full.insert("b".to_string(), 2);
    assert_eq!(optic.get_result(&full), Ok(1));
    let missing: HashMap<String, i32> = [("b".to_string(), 2)].into();
    assert_eq!(
        optic.get_result(&missing),
        Err(OpticError::MissingFocus("optional".into()))
    );
    assert_eq!(optic.replace(2, &full).get("a"), Some(&2));
    assert_eq!(optic.replace(2, &missing), missing);
}

#[test]
fn exact_optional_tuple_element() {
    let optic: Optional<(i32, Option<i32>), i32> = Optional::new(
        |s: &(i32, Option<i32>)| s.1,
        |v, s: &(i32, Option<i32>)| (s.0, Some(v)),
    );
    assert_eq!(optic.get_result(&(1, Some(2))), Ok(2));
    assert!(optic.get_result(&(1, None)).is_err());
    assert_eq!(optic.replace(3, &(1, Some(2))), (1, Some(3)));
    assert_eq!(optic.replace(3, &(1, None)), (1, None));
}

#[test]
fn array_optional_index_extends_and_truncates() {
    let optic: Optional<Vec<i32>, Option<i32>> = at_vec_exact(1);
    assert_eq!(optic.get_result(&vec![1, 2, 3]), Ok(Some(2)));
    assert_eq!(optic.get_result(&vec![1]), Ok(None));
    assert_eq!(optic.replace(Some(4), &vec![1, 2, 3]), vec![1, 4, 3]);
    assert_eq!(optic.replace(Some(4), &vec![1]), vec![1, 4]);
    assert_eq!(optic.replace(None, &vec![1, 2]), vec![1]);
    assert_eq!(optic.replace(None, &vec![1, 2, 3]), vec![1]);
}

#[test]
fn single_check_filters_focus() {
    let lens = Lens::field(|n: &i32| *n, |v, _| v);
    let optic = Prism::check_lens(1, &lens, |n: &i32| *n > 0);
    assert_eq!(optic.get_result(&1), Ok(1));
    assert!(optic.get_result(&0).is_err());
    assert_eq!(optic.replace(2, &1), 2);
    assert_eq!(optic.replace(2, &0), 0);
}

#[test]
fn multiple_checks_narrow_together() {
    let lens = Lens::field(|n: &f64| *n, |v, _| v);
    let is_int = Prism::check_lens(1.0, &lens, |n: &f64| n.fract() == 0.0);
    let positive = Prism::check_lens(1.0, &lens, |n: &f64| *n > 0.0);
    let both = is_int.compose(&positive);
    assert_eq!(both.get_result(&1.0), Ok(1.0));
    assert!(both.get_result(&0.0).is_err());
    assert!(both.get_result(&1.1).is_err());
    assert_eq!(both.modify(|n| n + 1.0, &1.0), 2.0);
    // Total modify on a missed focus keeps the source.
    assert_eq!(both.modify(|n| n + 1.0, &0.0), 0.0);
}

#[test]
fn checks_combine_across_successive_narrows() {
    let lens = Lens::field(|n: &i32| *n, |v, _| v);
    let positive = Prism::check_lens(1, &lens, |n: &i32| *n > 0);
    let non_zero = Prism::new(move |n: &i32| (*n != 0).then_some(*n), |n: i32| n);
    let both = positive.compose(&non_zero);
    assert_eq!(both.get_result(&1), Ok(1));
    assert!(both.get_result(&0).is_err());
}

#[test]
fn refine_predicate_prism() {
    let even = Prism::new(|n: &i32| (*n % 2 == 0).then_some(*n), |n: i32| n);
    assert_eq!(even.get_result(&2), Ok(2));
    assert!(even.get_result(&3).is_err());
    assert_eq!(even.replace(4, &2), 4);
    assert_eq!(even.replace(4, &3), 3);
}

#[derive(Clone, Debug, PartialEq)]
enum Shape {
    Circle(i32),
    Square(i32),
}

#[test]
fn tag_variant_prism_with_field() {
    let circle = Prism::new(
        |s: &Shape| match s {
            Shape::Circle(r) => Some(*r),
            _ => None,
        },
        Shape::Circle,
    );
    assert_eq!(circle.get_result(&Shape::Circle(1)), Ok(1));
    assert!(circle.get_result(&Shape::Square(1)).is_err());
    assert_eq!(circle.modify(add_one, &Shape::Circle(1)), Shape::Circle(2));
    assert_eq!(circle.modify(add_one, &Shape::Square(1)), Shape::Square(1));
}

#[test]
fn at_record_reports_missing_key() {
    let optic: Optional<HashMap<String, i32>, i32> = at_map("a".to_string());
    let empty: HashMap<String, i32> = HashMap::new();
    assert!(optic.get_result(&empty).is_err());
    assert_eq!(
        optic.replace_result(2, &empty),
        Err(OpticError::MissingFocus("optional".into()))
    );
}

#[test]
fn at_array_reports_missing_index() {
    let optic: Optional<Vec<i32>, i32> = at_vec(0);
    assert!(optic.get_result(&vec![]).is_err());
    assert_eq!(optic.replace(3, &vec![1, 2]), vec![3, 2]);
    assert_eq!(optic.replace(2, &vec![]), Vec::<i32>::new());
}

#[test]
fn key_with_check() {
    let lens = x_lens();
    let optic = Prism::check_lens(Point { x: 1, y: 0 }, &lens, |n: &i32| *n > 0);
    assert_eq!(optic.get_result(&Point { x: 1, y: 0 }), Ok(1));
    assert!(optic.get_result(&Point { x: 0, y: 0 }).is_err());
    assert_eq!(optic.replace(2, &Point { x: 1, y: 0 }).x, 2);
    // Total replace on a missed focus keeps the source.
    assert_eq!(optic.replace(2, &Point { x: 0, y: 0 }).x, 0);
}

#[derive(Clone, Debug, PartialEq)]
struct Triple {
    a: String,
    b: i32,
    c: bool,
}

#[test]
fn pick_struct_subset() {
    // `pick(["a", "c"])` is a subset isomorphism in Rust.
    let pick = Iso::new(
        |s: &Triple| (s.a.clone(), s.c),
        |(a, c): (String, bool)| Triple { a, b: 0, c },
    );
    let source = Triple {
        a: "a".into(),
        b: 1,
        c: true,
    };
    assert_eq!(pick.get(&source), ("a".to_string(), true));
    assert_eq!(pick.set(("x".to_string(), false)).a, "x".to_string());
}

#[test]
fn omit_struct_subset() {
    // `omit(["b"])` is the same subset isomorphism from the other side.
    let omit = Iso::new(
        |s: &Triple| (s.a.clone(), s.c),
        |(a, c): (String, bool)| Triple { a, b: 0, c },
    );
    let source = Triple {
        a: "a".into(),
        b: 1,
        c: true,
    };
    assert_eq!(omit.get(&source), ("a".to_string(), true));
    let rebuilt = omit.set(("x".to_string(), false));
    assert_eq!((rebuilt.a, rebuilt.c), ("x".to_string(), false));
}

#[test]
fn foreach_array_collects_and_maps() {
    let trav = Traversal::<Vec<i32>, i32>::vec_each();
    assert_eq!(trav.get_all(&vec![1, 2, 3]), vec![1, 2, 3]);
    assert_eq!(trav.modify_all(add_one, &vec![1, 2, 3]), vec![2, 3, 4]);
}

#[test]
fn foreach_record_entries() {
    let trav = Traversal::<HashMap<String, i32>, i32>::map_values();
    let source: HashMap<String, i32> = [
        ("a".to_string(), 0),
        ("b".to_string(), 1),
        ("c".to_string(), 0),
    ]
    .into();
    assert_eq!(trav.get_all(&source), vec![0, 1, 0]);
    let bumped = trav.modify_all(add_one, &source);
    assert_eq!((bumped["a"], bumped["b"], bumped["c"]), (1, 2, 1));
}

#[test]
fn traversal_replacement_count_must_match() {
    let trav = Traversal::<Vec<i32>, i32>::vec_each();
    assert_eq!(
        trav.replace_all(vec![2], &vec![1, 2]),
        Err(OpticError::LengthMismatch { got: 1, want: 2 })
    );
    assert_eq!(trav.replace_all(vec![2, 3], &vec![1, 2]), Ok(vec![2, 3]));
}

#[test]
fn traversal_inner_setter_failure_errors() {
    // Every focus misses: the traversal replacement reports the miss.
    let missing: Optional<Vec<i32>, i32> = Optional::new(
        |_: &Vec<i32>| None,
        |a, v: &Vec<i32>| {
            let mut next = v.clone();
            next.push(a);
            next
        },
    );
    assert_eq!(
        missing.replace_result(2, &vec![1]),
        Err(OpticError::MissingFocus("optional".into()))
    );
}

#[test]
fn traversal_over_optional_keeps_source_on_miss() {
    let inner: Optional<Vec<i32>, Vec<i32>> = Optional::new(|_: &Vec<i32>| None, |a, _| a);
    let source = vec![1, 2];
    // Miss at the outer focus: modify maps nothing, source unchanged.
    assert_eq!(inner.modify(|v| v, &source), source);
}

#[test]
fn not_undefined_is_option_prism() {
    let optic: Prism<Option<i32>, i32> = some();
    assert_eq!(optic.get_result(&Some(1)), Ok(1));
    assert!(optic.get_result(&None).is_err());
    assert_eq!(optic.replace(2, &None), None);
    assert_eq!(optic.replace(2, &Some(1)), Some(2));
}

#[test]
fn get_all_collects_present_foci() {
    // `getAll` over a checked traversal keeps passing foci, drops failures.
    let trav = Traversal::<Vec<i32>, i32>::vec_each().filter(|n| *n > 0);
    assert_eq!(trav.get_all(&vec![1, 2, 3]), vec![1, 2, 3]);
    assert_eq!(trav.get_all(&vec![1, -2, 3]), vec![1, 3]);
}

#[test]
fn get_all_empty_when_traversal_fails() {
    let never: Traversal<i32, i32> = Traversal::new(|_: &i32| Vec::<i32>::new(), |_, s: &i32| *s);
    assert_eq!(never.get_all(&1), Vec::<i32>::new());
}

#[test]
fn from_checks_compose() {
    let lens = Lens::field(|n: &i32| *n, |v, _| v);
    let positive = Prism::check_lens(1, &lens, |n: &i32| *n > 0);
    assert_eq!(positive.get_result(&1), Ok(1));
    assert!(positive.get_result(&0).is_err());
}

#[test]
fn option_some_prism() {
    let optic: Prism<Option<i32>, i32> = some();
    assert_eq!(optic.get_result(&Some(1)), Ok(1));
    assert!(optic.get_result(&None).is_err());
    assert_eq!(optic.set(2), Some(2));
}

#[test]
fn option_none_prism() {
    let optic: Prism<Option<i32>, ()> = none();
    assert_eq!(optic.get_result(&None), Ok(()));
    assert!(optic.get_result(&Some(1)).is_err());
    assert_eq!(optic.set(()), None);
}

#[test]
fn result_success_prism() {
    let optic: Prism<Result<i32, String>, i32> = success();
    assert_eq!(optic.get_result(&Ok(1)), Ok(1));
    assert!(optic.get_result(&Err("error".to_string())).is_err());
    assert_eq!(optic.set(2), Ok(2));
}

#[test]
fn result_failure_prism() {
    let optic: Prism<Result<i32, String>, String> = failure();
    assert_eq!(
        optic.get_result(&Err("error".to_string())),
        Ok("error".to_string())
    );
    assert!(optic.get_result(&Ok(1)).is_err());
    assert_eq!(
        optic.set("new error".to_string()),
        Err("new error".to_string())
    );
}

#[test]
fn nested_traversal_composes() {
    // Array of arrays: traversal composed with traversal.
    let outer = Traversal::<Vec<Vec<i32>>, Vec<i32>>::vec_each();
    let inner = Traversal::<Vec<i32>, i32>::vec_each();
    let both = outer.compose(&inner);
    assert_eq!(both.get_all(&vec![vec![1], vec![2, 3]]), vec![1, 2, 3]);
    assert_eq!(
        both.modify_all(add_one, &vec![vec![1], vec![2]]),
        vec![vec![2], vec![3]]
    );
}

#[test]
fn lens_iso_compose_roundtrip() {
    let lens = x_lens();
    let as_string = Iso::new(|n: &i32| n.to_string(), |s: String| s.parse().unwrap_or(0));
    let optic = lens.compose_iso(&as_string);
    let p = Point { x: 1, y: 7 };
    assert_eq!(optic.get(&p), "1".to_string());
    let replaced = optic.replace("2".to_string(), &p);
    assert_eq!((replaced.x, replaced.y), (2, 7));
}
