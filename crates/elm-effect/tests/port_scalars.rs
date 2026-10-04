//! Wave-4 scalars: Option, Result, UndefinedOr, Boolean, Number, String,
//! Brand behavior ported from the tag's suites. Modules load via `#[path]`
//! includes (the .15 precedent): no facade touch, no scope leak into lib.rs.
//!
//! Each test pins a behavior GROUP; the ledger maps every covered row to its
//! test. Residuals (do-notation, .17-owned Equal/Order/Hash/Function rows,
//! .19 Reducer rows, regex shapes) stay `unported` and are listed in the
//! close report instead of follow-up beads.

#[path = "../src/data/boolean.rs"]
mod boolean;
#[path = "../src/data/brand.rs"]
mod brand;
#[path = "../src/data/number.rs"]
mod number;
#[path = "../src/data/option.rs"]
mod option;
#[path = "../src/data/result.rs"]
mod result;
#[path = "../src/data/string.rs"]
mod string;
#[path = "../src/data/undefined_or.rs"]
mod undefined_or;

// ---------- Option ----------

#[test]
fn option_constructors() {
    assert_eq!(None::<i32>, None);
    assert_eq!(Some(1), Some(1));
    assert!(None::<i32>.is_none());
    assert!(Some(1).is_some());
    // isOption: every Option value is one (None and Some both classify).
    assert!(matches!(None::<i32>, None | Some(_)));
    assert_eq!(Some(()), Some(()));
}

#[test]
fn option_map_flatmap_flatten() {
    assert_eq!(Some(2).map(|a| a * 2), Some(4));
    assert_eq!(None::<i32>.map(|a| a * 2), None);
    assert_eq!(Some(2).and_then(|a| Some(a + 1)), Some(3));
    assert_eq!(None::<i32>.and_then(|_| Some(1)), None);
    assert_eq!(Some(Some(1)).flatten(), Some(1));
    assert_eq!(option::as_(Some(1), "x"), Some("x"));
    assert_eq!(option::as_(None::<i32>, "x"), None);
    assert_eq!(option::as_void(Some(1)), Some(()));
    assert_eq!(option::as_void(None::<i32>), None);
}

#[test]
fn option_unwrap() {
    assert_eq!(Some(1).unwrap_or_else(|| 2), 1);
    assert_eq!(None.unwrap_or_else(|| 2), 2);
    assert_eq!(option::get_or_throw(Some(7)), 7);
    assert_eq!(option::get_or_throw_with(None::<i32>, || "e"), Err("e"));
    assert_eq!(option::to_json(&Some(1)), "1");
    assert_eq!(option::to_json(&None::<i32>), "null");
}

#[test]
fn option_or_else_family() {
    assert_eq!(None.or_else(|| Some(2)), Some(2));
    assert_eq!(Some(1).or_else(|| Some(2)), Some(1));
    assert_eq!(option::or_else_some(None, 9), Some(9));
    assert_eq!(option::or_else_some(Some(1), 9), Some(1));
}

#[test]
fn option_filter_family() {
    assert_eq!(Some(4).filter(|a| a % 2 == 0), Some(4));
    assert_eq!(Some(3).filter(|a| a % 2 == 0), None);
    assert_eq!(
        Some(4).and_then(|a| (a % 2 == 0).then_some(a * 10)),
        Some(40)
    );
    assert_eq!(Some(1), Some(1));
    assert!(Some(1).is_some_and(|a| a > 0));
    let lift = option::lift_predicate(|a: &i32| *a > 0);
    assert_eq!(lift(1), Some(1));
    assert_eq!(lift(-1), None);
    let refine = option::to_refinement(|a: &i32| (*a > 0).then_some(*a));
    assert!(refine(&1));
    assert!(!refine(&-1));
}

#[test]
fn option_zip_product_all() {
    assert_eq!(option::zip_with(Some(2), Some(3), |a, b| a + b), Some(5));
    assert_eq!(option::zip_with(None::<i32>, Some(3), |a, b| a + b), None);
    assert_eq!(option::product(Some(1), Some(2)), Some((1, 2)));
    assert_eq!(option::tuple(Some(1), None::<i32>), None);
    assert_eq!(option::all(vec![Some(1), Some(2)]), Some(vec![1, 2]));
    assert_eq!(option::all(vec![Some(1), None]), None);
    assert_eq!(option::first_some_of(vec![None, Some(2), Some(3)]), Some(2));
    assert_eq!(option::first_some_of(vec![None::<i32>, None]), None);
    assert_eq!(option::from_iterable(vec![1, 2]), Some(1));
    assert_eq!(option::from_iterable(Vec::<i32>::new()), None);
}

#[test]
fn option_match_tap() {
    assert_eq!(option::match_(Some(1), || 0, |a| a + 1), 2);
    assert_eq!(option::match_(None::<i32>, || 0, |a| a + 1), 0);
    let mut seen = 0;
    let _ = Some(5).inspect(|a| seen = *a);
    assert_eq!(seen, 5);
    let mut tapped = 0;
    assert_eq!(option::tap(Some(6), |a| tapped = *a), Some(6));
    assert_eq!(tapped, 6);
    assert_eq!(Some(1).into_iter().collect::<Vec<_>>(), vec![1]);
    assert_eq!(option::lift_throwable(|| 1), Some(1));
    assert_eq!(option::lift_throwable(|| -> i32 { panic!("boom") }), None);
    assert_eq!(option::from_nullish(Some(Some(1))), Some(1));
    assert_eq!(
        option::flat_map_nullish(Some(2), |a| Some(Some(a * 2))),
        Some(4)
    );
    assert_eq!(
        option::flat_map_nullish(None::<i32>, |a| Some(Some(a))),
        None
    );
    let lifted = option::lift_nullish_or(|a: i32| if a > 0 { Some(a) } else { None });
    assert_eq!(lifted(1), Some(1));
    assert_eq!(lifted(-1), None);
    assert_eq!(option::get_success(Ok::<i32, &str>(1)), Some(1));
    assert_eq!(option::get_failure(Err::<i32, &str>("e")), Some("e"));
}

// ---------- Result ----------

#[test]
fn result_constructors() {
    assert_eq!(result::succeed(1), Ok::<i32, std::convert::Infallible>(1));
    assert_eq!(
        result::fail("e"),
        Err::<std::convert::Infallible, &str>("e")
    );
    assert!(result::is_success(&Ok::<i32, &str>(1)));
    assert!(!result::is_success(&Err::<i32, &str>("e")));
    assert!(Err::<i32, &str>("e").is_err());
    assert!(Ok::<i32, &str>(1).is_ok());
}

#[test]
fn result_map_family() {
    assert_eq!(Ok::<i32, &str>(1).map(|a| a + 1), Ok(2));
    assert_eq!(
        result::map_both(Ok::<i32, &str>(1), |a| a + 1, |e: &str| e.len()),
        Ok::<i32, usize>(2)
    );
    assert_eq!(
        result::map_both(Err::<i32, &str>("e"), |a| a + 1, |e: &str| e.len()),
        Err::<i32, usize>(1)
    );
    assert_eq!(Err::<i32, &str>("e").map_err(|e| e.len()), Err(1));
    assert_eq!(result::flip(Ok::<i32, &str>(1)), Err(1));
    assert_eq!(result::flip(Err::<i32, &str>("e")), Ok("e"));
    assert_eq!(result::merge(Ok::<i32, i32>(1)), 1);
    assert_eq!(result::merge(Err::<i32, i32>(2)), 2);
}

#[test]
fn result_chain() {
    assert_eq!(Ok::<i32, &str>(1).and_then(|a| Ok(a + 1)), Ok(2));
    assert_eq!(Err::<i32, &str>("e").and_then(|a: i32| Ok(a + 1)), Err("e"));
    assert_eq!(Err::<i32, &str>("e").or_else(|_| Ok::<i32, &str>(1)), Ok(1));
    assert_eq!(result::match_(Ok::<i32, &str>(1), |_| 0, |a| a + 1), 2);
    assert_eq!(result::match_(Err::<i32, &str>("e"), |_| 0, |a| a + 1), 0);
    assert_eq!(
        result::filter_or_fail(Ok::<i32, String>(4), |a| a % 2 == 0, |a| format!("odd {a}")),
        Ok(4)
    );
    assert!(
        result::filter_or_fail(
            Ok::<i32, String>(3),
            |a| *a % 2 == 0,
            |a| format!("odd {a}")
        )
        .is_err()
    );
    assert_eq!(
        result::from_option(Some(1), || "missing"),
        Ok::<i32, &str>(1)
    );
    assert_eq!(
        result::from_option(None::<i32>, || "missing"),
        Err("missing")
    );
    assert_eq!(
        result::transpose_option(Ok::<Option<i32>, &str>(Some(1))),
        Some(Ok(1))
    );
    assert_eq!(
        result::transpose_option(Ok::<Option<i32>, &str>(None)),
        None
    );
    assert_eq!(
        result::transpose_option(Err::<Option<i32>, &str>("e")),
        Some(Err("e"))
    );
}

#[test]
fn result_unwrap() {
    assert_eq!(Ok::<i32, &str>(1).unwrap_or_else(|_| 0), 1);
    assert_eq!(Err::<i32, &str>("e").unwrap_or_else(|_| 0), 0);
    assert_eq!(result::get_or_throw(Ok::<i32, String>(5)), 5);
    assert_eq!(
        result::get_or_throw_with(Err::<(), &str>("e"), |e| e.len()),
        Err(1)
    );
    let mut seen = 0;
    let _ = Ok::<i32, &str>(3).inspect(|a| seen = *a);
    assert_eq!(seen, 3);
    let mut tapped = 0;
    assert_eq!(result::tap(Ok::<i32, &str>(4), |a| tapped = *a), Ok(4));
    assert_eq!(tapped, 4);
    assert_eq!(result::try_(|| 1), Ok(1));
    assert!(result::try_(|| -> i32 { panic!("x") }).is_err());
    assert_eq!(result::void(Ok::<i32, &str>(1)), Ok(()));
    assert_eq!(result::to_json(&Ok::<i32, &str>(1)), "1");
    assert_eq!(result::to_json(&Err::<i32, &str>("e")), "Failure(e)");
}

// ---------- UndefinedOr ----------

#[test]
fn undefined_or_behaves_as_option() {
    let some: undefined_or::UndefinedOr<i32> = Some(1);
    let none: undefined_or::UndefinedOr<i32> = None;
    assert_eq!(some.map(|a| a + 1), Some(2));
    assert_eq!(none.map(|a| a + 1), None);
    assert_eq!(undefined_or::get_or_throw(Some(3)), 3);
    assert_eq!(
        undefined_or::get_or_throw_with(None::<i32>, || "gone"),
        Err("gone")
    );
    assert_eq!(undefined_or::lift_throwable(|| 2), Some(2));
}

// ---------- Boolean ----------

#[test]
fn boolean_ops() {
    // and / or / xor / implies truth tables.
    for (a, b) in [(false, false), (false, true), (true, false), (true, true)] {
        assert_eq!(boolean::and_(a, b), a && b);
        assert_eq!(boolean::or_(a, b), a || b);
        assert_eq!(boolean::xor(a, b), a ^ b);
        assert_eq!(boolean::implies(a, b), !a || b);
    }
    assert!(!boolean::not(true));
    assert!(boolean::not(false));
    // not is self-inverse.
    for b in [false, true] {
        assert_eq!(boolean::not(boolean::not(b)), b);
    }
    // false < true ordering (via variables: literals trip absurd-comparison).
    let (t, f) = (true, false);
    assert!(f < t);
    assert!(!(t < f));
}

#[test]
fn boolean_match() {
    // onTrue / onFalse in both data-first and data-last shapes (same fn).
    assert_eq!(boolean::match_bool(true, || 0, || 1), 1);
    assert_eq!(boolean::match_bool(false, || 0, || 1), 0);
    let on_true = || "yes";
    let on_false = || "no";
    assert_eq!(boolean::match_bool(true, on_false, on_true), "yes");
    assert_eq!(boolean::match_bool(false, on_false, on_true), "no");
}

#[test]
fn boolean_coerce() {
    // isBoolean narrows; the constructor coerces. Both are identity on bool.
    assert!(boolean::is_boolean(true));
    assert!(boolean::is_boolean(false));
    assert!(boolean::coerce(true));
    assert!(!boolean::coerce(false));
    assert_eq!(bool::default(), false);
    assert_ne!(true, false);
}

#[test]
fn boolean_all_any() {
    assert!(boolean::all(vec![true, true]));
    assert!(!boolean::all(vec![true, false]));
    assert!(boolean::all(Vec::<bool>::new()));
    assert!(boolean::any(vec![false, true]));
    assert!(!boolean::any(vec![false, false]));
    assert!(!boolean::any(Vec::<bool>::new()));
    // Short-circuit: infinite iterators terminate.
    assert!(!boolean::all(
        [true, true].into_iter().chain(std::iter::repeat(false))
    ));
    assert!(boolean::any(
        [false, false].into_iter().chain(std::iter::repeat(true))
    ));
    // Equal booleans and negation-of-xor shape.
    assert_eq!(true, true);
    assert_ne!(true, false);
    assert_eq!(true.cmp(&true), std::cmp::Ordering::Equal);
    assert_eq!(false.cmp(&true), std::cmp::Ordering::Less);
    for (a, b) in [(false, false), (false, true), (true, false), (true, true)] {
        assert_eq!(!(a ^ b), a == b);
    }
}

// ---------- Number ----------

#[test]
fn number_parse() {
    assert_eq!(number::parse("42"), Some(42.0));
    assert_eq!(number::parse("  3.5  "), Some(3.5));
    assert_eq!(number::parse("junk"), None);
    assert_eq!(number::parse(""), None);
    assert!(number::parse("NaN").unwrap().is_nan());
    assert!(number::is_number(1.0));
    assert!(number::is_number(f64::NAN));
}

#[test]
fn number_arithmetic() {
    assert_eq!(number::add(2.0, 3.0), 5.0);
    assert_eq!(number::sub(5.0, 3.0), 2.0);
    assert_eq!(number::mul(2.0, 3.0), 6.0);
    assert_eq!(number::div(7.0, 2.0), 3.5);
    assert_eq!(number::remainder(7.0, 3.0), 1.0);
    assert_eq!(number::remainder(7.5, 2.0), 1.5);
    assert_eq!(number::increment(1.0), 2.0);
    assert_eq!(number::decrement(1.0), 0.0);
}

#[test]
fn number_round() {
    assert_eq!(number::round(2.5), 3.0);
    assert_eq!(number::round(2.4), 2.0);
    // Half-up differs from f64::round (half away from zero) on negatives.
    assert_eq!(number::round(-2.5), -2.0);
    assert_eq!(number::floor(2.9), 2.0);
    assert_eq!(number::ceil(2.1), 3.0);
}

#[test]
fn number_range() {
    assert_eq!(number::clamp(5.0, 0.0, 10.0), 5.0);
    assert_eq!(number::clamp(-5.0, 0.0, 10.0), 0.0);
    assert_eq!(number::clamp(15.0, 0.0, 10.0), 10.0);
    assert_eq!(number::min(2.0, 3.0), 2.0);
    assert_eq!(number::max(2.0, 3.0), 3.0);
    assert!(number::between(5.0, 0.0, 10.0));
    assert!(number::between(0.0, 0.0, 10.0));
    assert!(!number::between(11.0, 0.0, 10.0));
    assert_eq!(number::sign(5.0), 1.0);
    assert_eq!(number::sign(-5.0), -1.0);
    assert_eq!(number::sign(0.0), 0.0);
}

#[test]
fn number_aggregate() {
    assert_eq!(number::sum(&[1.0, 2.0, 3.0]), 6.0);
    assert_eq!(number::sum(&[]), 0.0);
    assert_eq!(number::avg(&[1.0, 2.0, 3.0]), Some(2.0));
    assert_eq!(number::avg(&[]), None);
    assert_eq!(number::abs(-3.0), 3.0);
    assert_eq!(number::sqrt(9.0), 3.0);
}

// ---------- String ----------

#[test]
fn string_basics() {
    assert!(string::is_string("x"));
    assert_eq!(string::length("hello"), 5);
    assert_eq!(string::length(""), 0);
    // Astral chars count once (documented UTF-16 divergence).
    assert_eq!(string::length("a💯b"), 3);
    assert_eq!(string::lines("a\nb\nc"), vec!["a", "b", "c"]);
    assert_eq!("  x  ".trim(), "x");
    assert_eq!("  x  ".trim_start(), "x  ");
    assert_eq!("  x  ".trim_end(), "  x");
    assert!("hello".starts_with("he"));
    assert!("hello".ends_with("lo"));
    assert!("hello".contains("ll"));
    assert_eq!("a,b".split(',').collect::<Vec<_>>(), vec!["a", "b"]);
    assert_eq!("aaa".replace("a", "b"), "bbb");
    assert_eq!("hello".to_uppercase(), "HELLO");
    assert_eq!("HELLO".to_lowercase(), "hello");
    assert_eq!(["a", "b"].concat(), "ab");
}

#[test]
fn string_shapes() {
    assert_eq!(string::take_left("hello", 2), "he");
    assert_eq!(string::take_right("hello", 2), "lo");
    assert_eq!(string::drop_left("hello", 2), "llo");
    assert_eq!(string::drop_right("hello", 2), "hel");
    assert_eq!(string::slice_chars("hello", 1, 4), "ell");
    assert_eq!(string::capitalize("hello"), "Hello");
    assert_eq!(string::capitalize(""), "");
    assert_eq!(string::pad_start("5", 3, '0'), "005");
    assert_eq!(string::pad_end("5", 3, '0'), "500");
    assert_eq!(string::pad_start("hello", 3, '0'), "hello");
    assert_eq!(string::ellipsis("hello world", 8), "hello w…");
    assert_eq!(string::ellipsis("hi", 8), "hi");
    assert_eq!(string::strip_margin("  |a\n  |b"), "a\nb");
    assert_eq!("xy".repeat(0), "");
}

// ---------- Brand ----------

#[test]
fn brand_nominal_and_make() {
    #[derive(Debug)]
    struct UserId;
    // Nominal: no runtime validation.
    let id: brand::Branded<i32, UserId> = brand::nominal(7);
    assert_eq!(id.into_value(), 7);
    // Custom predicate via make.
    let ok: Result<brand::Branded<i32, UserId>, brand::BrandError> =
        brand::make(5, |a: &i32| *a > 0, "positive");
    assert!(ok.is_ok());
    let err: Result<brand::Branded<i32, UserId>, brand::BrandError> =
        brand::make(-1, |a: &i32| *a > 0, "positive");
    assert_eq!(
        err.unwrap_err().to_string(),
        "BrandError: predicate positive failed"
    );
}

#[test]
fn brand_validate_accumulates_and_aborts() {
    use brand::Check;
    #[derive(Debug)]
    struct Even;
    let check_even = |a: &i32| {
        if a % 2 == 0 {
            Check::Pass
        } else {
            Check::Fail("not even".to_string())
        }
    };
    let check_small = |a: &i32| {
        if *a < 100 {
            Check::Pass
        } else {
            Check::Fail("too big".to_string())
        }
    };
    // Single schema check passes.
    assert!(brand::refined::<i32, Even>(4, check_even).is_ok());
    // Single failure.
    let one = brand::refined::<i32, Even>(3, check_even).unwrap_err();
    assert_eq!(one.len(), 1);
    // Multiple failures accumulate.
    let many = brand::validate::<i32, Even>(
        101,
        &[
            Box::new(check_even) as Box<dyn Fn(&i32) -> Check>,
            Box::new(check_small),
        ],
    )
    .unwrap_err();
    assert_eq!(many.len(), 2);
    // Abort stops the rest.
    let aborting = brand::validate::<i32, Even>(
        101,
        &[
            Box::new(|_: &i32| Check::Abort("stop".to_string())) as Box<dyn Fn(&i32) -> Check>,
            Box::new(check_small),
        ],
    )
    .unwrap_err();
    assert_eq!(aborting.len(), 1);
    assert_eq!(aborting[0].to_string(), "BrandError: stop");
}
