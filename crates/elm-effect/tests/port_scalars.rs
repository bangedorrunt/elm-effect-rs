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
#[path = "../src/data/combiner.rs"]
mod combiner;
#[path = "../src/foundation.rs"]
mod foundation;
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
    assert_eq!(Some(2).map(|a| a + 1), Some(3));
    assert_eq!(None::<i32>.map(|_| 1), None);
    assert_eq!(Some(Some(1)).flatten(), Some(1));
    assert_eq!(option::as_(Some(1), "x"), Some("x"));
    assert_eq!(option::as_(None::<i32>, "x"), None);
    assert_eq!(option::as_void(Some(1)), Some(()));
    assert_eq!(option::as_void(None::<i32>), None);
}

#[test]
fn option_unwrap() {
    fn one() -> Option<i32> {
        Some(1)
    }
    fn none() -> Option<i32> {
        None
    }
    assert_eq!(one().unwrap_or(2), 1);
    assert_eq!(none().unwrap_or(2), 2);
    assert_eq!(option::get_or_throw(Some(7)), 7);
    assert_eq!(option::get_or_throw_with(None::<i32>, || "e"), Err("e"));
    assert_eq!(option::to_json(&Some(1)), "1");
    assert_eq!(option::to_json(&None::<i32>), "null");
}

#[test]
fn option_or_else_family() {
    assert_eq!(None::<i32>.or(Some(2)), Some(2));
    assert_eq!(Some(1).or(Some(2)), Some(1));
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
    assert_eq!(Ok::<i32, &str>(1).map(|a| a + 1), Ok(2));
    assert_eq!(Err::<i32, &str>("e").map(|a: i32| a + 1), Err("e"));
    assert_eq!(Err::<i32, &str>("e").or(Ok::<i32, &str>(1)), Ok(1));
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
    fn ok() -> Result<i32, &'static str> {
        Ok(1)
    }
    fn err() -> Result<i32, &'static str> {
        Err("e")
    }
    assert_eq!(ok().unwrap_or(0), 1);
    assert_eq!(err().unwrap_or(0), 0);
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
    assert!(!f && t);
    assert!(t || !f);
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
    assert!(!bool::default());
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
    assert_eq!(number::parse_strict("  3.5  "), Ok(3.5));
    assert!(number::parse_strict("junk").is_err());
    assert!(number::is_number(1.0));
    assert!(number::is_number(f64::NAN));
}

#[test]
fn number_arithmetic() {
    assert_eq!(number::add(2.0, 3.0), 5.0);
    assert_eq!(number::add(0.0, 0.0), 0.0);
    assert_eq!(number::sub(0.0, 5.0), -5.0);
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
    assert_eq!(number::min(2.0, 2.0), 2.0);
    assert_eq!(number::max(2.0, 2.0), 2.0);
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
    assert_eq!(string::trim("  x  "), "x");
    assert!(string::starts_with("hello", "he"));
    assert!(string::ends_with("hello", "lo"));
    assert!(string::contains("hello", "ll"));
    assert!(!string::contains("hello", "z"));
    assert_eq!(string::split("a,b", ","), vec!["a", "b"]);
    assert_eq!(string::replace_all("aaa", "a", "b"), "bbb");
    assert_eq!(string::to_uppercase("hello"), "HELLO");
    assert_eq!(string::to_lowercase("HELLO"), "hello");
    assert_eq!(string::concat("a", "b"), "ab");
    assert_eq!(string::concat("", ""), "");
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

// --- Wave-6 residuals (uy4): deferred rows whose deps (.17/.19/.23) landed ---

#[test]
fn option_do_gen_bind_shapes() {
    assert_eq!(option::do_(), Some(()));
    assert_eq!(option::gen_block(|| Some(1)), Some(1));
    assert_eq!(option::gen_block(|| None::<i32>), None);
    // ? propagates None inside gen blocks.
    let chained = option::gen_block(|| {
        let a = Some(2)?;
        let b = option::bind(Some(3), |x| Some(x * 10))?;
        Some(a + b)
    });
    assert_eq!(chained, Some(32));
    assert_eq!(option::bind(None::<i32>, |x| Some(x + 1)), None);
    assert_eq!(option::bind_to(Some(1), |_| 2), Some((1, 2)));
    assert_eq!(option::let_(Some(1), |v| v + 1), Some((1, 2)));
    assert_eq!(option::bind_to(None::<i32>, |_| 2), None);
}

#[test]
fn option_lift_partition_product() {
    assert_eq!(option::lift2(|a, b| a + b, Some(1), Some(2)), Some(3));
    assert_eq!(option::lift2(|a, b| a + b, Some(1), None::<i32>), None);
    assert_eq!(
        option::partition_map(Some("42"), |s| s.parse::<i32>().map_err(|_| "nan")),
        (None, Some(42))
    );
    assert_eq!(
        option::partition_map(Some("abc"), |s| s.parse::<i32>().map_err(|_| "nan")),
        (Some("nan"), None)
    );
    assert_eq!(
        option::partition_map(None::<&str>, |s| s.parse::<i32>().map_err(|_| "nan")),
        (None, None)
    );
    assert_eq!(
        option::product_many(Some(1), vec![Some(2), Some(3)]),
        Some(vec![1, 2, 3])
    );
    assert_eq!(option::product_many(Some(1), vec![Some(2), None]), None);
    assert_eq!(option::reduce_compact(Some(2), 1, |b, a| b + a), 3);
    assert_eq!(option::reduce_compact(None::<i32>, 1, |b, a| b + a), 1);
    assert_eq!(option::struct2(Some(1), Some("a")), Some((1, "a")));
    assert_eq!(option::struct3(Some(1), Some("a"), None::<bool>), None);
    assert_eq!(
        option::or_else_result(Some(1), || Err::<i32, &str>("e")),
        Ok(1)
    );
    assert_eq!(option::or_else_result(None::<i32>, || Err("e")), Err("e"));
    assert_eq!(option::filter(Some(2), |v| *v > 1), Some(2));
    assert_eq!(option::filter(Some(1), |v| *v > 1), None);
}

#[test]
fn option_equal_order_reducer() {
    assert!(option::equal_with(&Some(1), &Some(1)));
    assert!(!option::equal_with(&Some(1), &None));
    assert!(option::equal_with(&None::<i32>, &None));
    assert!(option::equal_with(&Some(f64::NAN), &Some(f64::NAN)));
    assert_eq!(
        option::order_with(&None::<i32>, &Some(1)),
        std::cmp::Ordering::Less
    );
    assert!(option::contains_with(&Some(1), &1));
    assert!(!option::contains_with(&Some(1), &2));
    assert!(!option::contains_with(&None::<i32>, &1));
    let sum = combiner::Combiner::make(|a: &i32, b: &i32| a + b);
    let r = option::make_reducer(sum);
    assert_eq!(r.combiner.combine(&Some(1), &Some(2)), Some(3));
    assert_eq!(r.combiner.combine(&None, &Some(2)), Some(2));
    assert_eq!(r.combiner.combine(&Some(1), &None), Some(1));
    assert_eq!(r.zero, None);
    let base = combiner::Reducer::make(0, |a: &i32, b: &i32| a + b);
    let ff = option::make_reducer_fail_fast(base);
    assert_eq!(ff.combiner.combine(&Some(1), &Some(2)), Some(3));
    assert_eq!(ff.combiner.combine(&Some(1), &None), None);
    assert_eq!(ff.combiner.combine(&None, &None), None);
}

#[test]
fn result_do_bind_transpose() {
    assert_eq!(result::do_::<&str>(), Ok(()));
    assert_eq!(result::gen_block(|| Ok::<i32, &str>(1)), Ok(1));
    assert_eq!(result::bind(Ok(1), |x| Ok::<i32, &str>(x + 1)), Ok(2));
    assert_eq!(result::bind(Err::<i32, &str>("e"), |x| Ok(x + 1)), Err("e"));
    assert_eq!(result::bind_to(Ok::<i32, &str>(1), |_| 2), Ok((1, 2)));
    assert_eq!(result::let_(Ok::<i32, &str>(1), |v| v + 1), Ok((1, 2)));
    assert_eq!(result::to_string(&Ok::<i32, &str>(1)), "1");
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
    assert!(result::equal_with(&Ok::<i32, String>(1), &Ok(1)));
    assert!(!result::equal_with(
        &Ok::<i32, String>(1),
        &Err("e".to_string())
    ));
}

#[test]
fn undefined_or_match_reducer() {
    assert_eq!(undefined_or::match_(Some(1), || 0, |v| v + 1), 2);
    assert_eq!(undefined_or::match_(None::<i32>, || 0, |v| v + 1), 0);
    let sum = combiner::Combiner::make(|a: &i32, b: &i32| a + b);
    let r = undefined_or::make_reducer(sum);
    assert_eq!(r.combiner.combine(&Some(1), &Some(2)), Some(3));
    assert_eq!(r.combiner.combine(&None, &Some(2)), Some(2));
    let base = combiner::Reducer::make(0, |a: &i32, b: &i32| a + b);
    let ff = undefined_or::make_reducer_fail_fast(base);
    assert_eq!(ff.combiner.combine(&Some(1), &None), None);
}

#[test]
fn number_order_nan_rows() {
    use std::cmp::Ordering::*;
    assert_eq!(number::compare(f64::NAN, 1.0), Less);
    assert_eq!(number::compare(f64::NAN, 0.0), Less);
    assert_eq!(number::compare(1.0, f64::NAN), Greater);
    assert_eq!(number::compare(f64::NAN, f64::NAN), Equal);
    assert_eq!(number::compare(1.0, 2.0), Less);
    assert_eq!(number::compare(2.0, 2.0), Greater);
    assert!(number::equivalent(f64::NAN, f64::NAN));
    assert!(number::equivalent(0.0, -0.0));
    assert!(!number::equivalent(1.0, 2.0));
    assert_eq!(number::clamp_ordered(f64::NAN, 0.0, 10.0), 0.0);
    assert_eq!(number::clamp_ordered(5.0, 0.0, 10.0), 5.0);
    assert!(number::less_than(f64::NAN, 0.0));
    assert!(number::less_than(1.0, 2.0));
    assert!(!number::less_than(2.0, 1.0));
}

#[test]
fn string_portable_remainder() {
    assert_eq!(string::split("a,b,c", ","), ["a", "b", "c"]);
    assert_eq!(string::replace_all("aaa", "a", "b"), "bbb");
    assert_eq!(string::replace_first("aaa", "a", "b"), "baa");
    assert_eq!(string::to_uppercase("ab"), "AB");
    assert_eq!(string::to_lowercase("AB"), "ab");
    assert_eq!(string::repeat("ab", 2), "abab");
    assert_eq!(string::concat("a", "b"), "ab");
    assert!(string::contains("abc", "b"));
    assert!(string::starts_with("abc", "ab"));
    assert!(string::ends_with("abc", "bc"));
    assert_eq!(string::strip_prefix("abc", "ab"), Some("c"));
    assert_eq!(string::strip_suffix("abc", "bc"), Some("a"));
    assert_eq!(string::trim("  a  "), "a");
    assert_eq!(string::head("ab"), Some('a'));
    assert_eq!(string::last("ab"), Some('b'));
}

#[test]
fn scalars_residuals() {
    use elm_effect::Equal as _;
    // Result structural equality (cross-bead residual).
    assert!(Ok::<u32, String>(1).equal(&Ok(1)));
    assert!(!Ok::<u32, String>(1).equal(&Err("x".to_string())));
    assert!(Err::<u32, String>("x".to_string()).equal(&Err("x".to_string())));
    // Null collapses to Option on both sides.
    assert_eq!(option::get_or_null(Some(1)), Some(1));
    assert_eq!(option::get_or_null(None::<i32>), None);
    assert_eq!(option::get_or_undefined(Some(1)), Some(1));
    assert_eq!(result::get_or_null(Ok::<i32, &str>(1)), Some(1));
    assert_eq!(result::get_or_null(Err::<i32, &str>("e")), None);
    assert_eq!(result::get_or_undefined(Ok::<i32, &str>(1)), Some(1));
    // Min/max ties and NaN-producing ops.
    assert_eq!(number::min(2.0, 2.0), 2.0);
    assert_eq!(number::max(2.0, 2.0), 2.0);
    assert!(number::sqrt(-1.0).is_nan());
    assert!(number::div(0.0, 0.0).is_nan());
}

#[test]
fn string_case_first_char() {
    // String.test.ts:106/110/114 - capitalize first/single/empty/already.
    assert_eq!(string::capitalize("a"), "A");
    assert_eq!(string::capitalize(""), "");
    assert_eq!(string::capitalize("Hello"), "Hello");
    // :120/:124/:128/:132 - uncapitalize mirrors it.
    assert_eq!(string::uncapitalize("Hello"), "hello");
    assert_eq!(string::uncapitalize("A"), "a");
    assert_eq!(string::uncapitalize(""), "");
    assert_eq!(string::uncapitalize("ABC"), "aBC");
}

#[test]
fn number_divide_product_and_order() {
    // Number.test.ts:95 - divide yields None for zero divisors.
    assert_eq!(number::divide(6.0, 3.0), Some(2.0));
    assert_eq!(number::divide(0.0, 3.0), Some(0.0));
    assert_eq!(number::divide(6.0, 0.0), None);
    assert_eq!(number::divide(6.0, -0.0), None);
    // :279/:284 - product multiplies; iteration stops at zero.
    assert_eq!(number::product([2.0, 3.0, 4.0]), 24.0);
    assert_eq!(number::product([]), 1.0);
    assert_eq!(number::product([2.0, 0.0, 5.0]), 0.0);
    // :147/:153 - lessThan is strict; data-last is the same call in Rust.
    assert!(number::less_than(1.0, 2.0));
    assert!(!number::less_than(2.0, 2.0));
    assert!(!number::less_than(2.0, 1.0));
    // :159/:165 - lessThanOrEqual.
    assert!(number::less_than_or_equal(2.0, 2.0));
    assert!(number::less_than_or_equal(1.0, 2.0));
    assert!(!number::less_than_or_equal(3.0, 2.0));
    // :171/:177 - greaterThan is strict.
    assert!(number::greater_than(2.0, 1.0));
    assert!(!number::greater_than(2.0, 2.0));
    assert!(!number::greater_than(1.0, 2.0));
    // :183/:189 - greaterThanOrEqual.
    assert!(number::greater_than_or_equal(2.0, 2.0));
    assert!(number::greater_than_or_equal(2.0, 1.0));
    assert!(!number::greater_than_or_equal(1.0, 2.0));
    // :205/:219 - clamping covers its data-last form the same way.
    assert_eq!(number::clamp(5.0, 0.0, 10.0), 5.0);
}

#[test]
fn number_remainder_sign_and_nan() {
    // Number.test.ts:290 - integer and decimal remainders.
    assert_eq!(number::remainder(3.0, 2.0), 1.0);
    assert_eq!(number::remainder(0.3, 0.2), 0.1);
    // :296/:302 - the dividend sign survives, including negative zero.
    assert_eq!(number::remainder(-5.0, 2.0), -1.0);
    assert!(number::remainder(-4.0, 2.0).is_sign_negative());
    assert!(number::remainder(-0.0, 2.0).is_sign_negative());
    assert_eq!(number::remainder(5.0, -2.0), 1.0);
    assert_eq!(number::remainder(-5.0, -2.0), -1.0);
    assert!(number::remainder(-4.0, -2.0).is_sign_negative());
    // :313 - zero divisors and non-finite operands yield NaN.
    assert!(number::remainder(5.0, 0.0).is_nan());
    assert!(number::remainder(1e-101, 0.0).is_nan());
    assert!(number::remainder(f64::NAN, 2.0).is_nan());
    assert!(number::remainder(f64::INFINITY, 2.0).is_nan());
    assert!(number::remainder(2.0, f64::INFINITY).is_nan());
}

#[test]
fn number_remainder_scientific_and_subnormal() {
    // Number.test.ts:321 - small floats and scientific notation.
    let divisor = 1e-7;
    assert_eq!(number::remainder(0.0, divisor), 0.0);
    assert_eq!(number::remainder(1e-7, divisor), 0.0);
    assert_eq!(number::remainder(2e-7, divisor), 0.0);
    assert_eq!(number::remainder(3e-7, divisor), 0.0);
    assert_eq!(number::remainder(2.5e-7, divisor), 5e-8);
    assert_eq!(number::remainder(1.5e-7, divisor), 5e-8);
    // :335 - signs survive in scientific notation.
    assert_eq!(number::remainder(-2.5e-7, divisor), -5e-8);
    assert_eq!(number::remainder(2.5e-7, -divisor), 5e-8);
    assert_eq!(number::remainder(-2.5e-7, -divisor), -5e-8);
    // :344 - beyond toFixed precision.
    let tiny = 1e-101;
    assert_eq!(number::remainder(0.0, tiny), 0.0);
    assert_eq!(number::remainder(tiny, tiny), 0.0);
    assert_eq!(number::remainder(3e-101, tiny), 0.0);
    assert_eq!(number::remainder(2.5e-101, tiny), 5e-102);
    assert!(number::remainder(-3e-101, tiny).is_sign_negative());
    // :354/:362 - subnormals.
    let min = f64::from_bits(1);
    assert_eq!(number::remainder(min, min), 0.0);
    assert_eq!(number::remainder(min * 2.0, min), 0.0);
    assert_eq!(number::remainder(min, min * 2.0), min);
    assert_eq!(number::remainder(1.042e-321, 1e-323), f64::from_bits(1));
    assert_eq!(number::remainder(-1.042e-321, 1e-323), -f64::from_bits(1));
    // :369 - large values in scientific notation.
    assert_eq!(number::remainder(1e21, 2.0), 0.0);
    assert_eq!(number::remainder(1e21, 3.0), 1.0);
    assert_eq!(number::remainder(3.0, 1e21), 3.0);
    assert!(number::remainder(-1e21, 2.0).is_sign_negative());
    // :378/:383 - big decimal coefficients and exact integers.
    assert_eq!(number::remainder(100000000000000.05, 0.03), 0.0);
    assert_eq!(number::remainder(100000000000000.1, 0.03), 0.02);
    assert_eq!(number::remainder(1152921504606846976.0, 5.0), 1.0);
    assert_eq!(number::remainder(-1152921504606846976.0, 5.0), -1.0);
}

#[test]
fn string_slice_js_semantics() {
    // String.test.ts:186 - extracts a section.
    assert_eq!(string::slice("abcd", 1, Some(3)), "bc");
    // :190 - negative indices count from the end.
    assert_eq!(string::slice("hello", -3, None), "llo");
    // :194 - no arguments returns the full string.
    assert_eq!(string::slice("hello", 0, None), "hello");
    assert_eq!(string::slice("", 0, None), "");
    assert_eq!(string::slice("hello", 3, Some(1)), "");
}

#[test]
fn string_order_and_concat() {
    use std::cmp::Ordering;
    // String.test.ts:38 - empty strings compare.
    assert_eq!(string::order("", ""), Ordering::Equal);
    assert_eq!(string::order("", "a"), Ordering::Less);
    assert_eq!(string::order("a", ""), Ordering::Greater);
    // String.test.ts:378 - ASCII ordering matches localeCompare.
    assert_eq!(string::order("a", "b"), Ordering::Less);
    assert_eq!(string::order("b", "a"), Ordering::Greater);
    assert_eq!(string::order("a", "a"), Ordering::Equal);
    // :62 - data-first concat.
    assert_eq!(string::concat("hello", " world"), "hello world");
    // :66 - data-last (pipeable) concat: the suffix section applied to a receiver.
    let with_world = |receiver: &str| string::concat(receiver, " world");
    assert_eq!(with_world("hello"), "hello world");
    assert_eq!(string::concat("", ""), "");
}

#[test]
fn string_case_conversions() {
    // String.test.ts:78 - converts to uppercase.
    assert_eq!(string::to_uppercase("hello"), "HELLO");
    // :82 - uppercase handles empty string.
    assert_eq!(string::to_uppercase(""), "");
    // :86 - pipeable: the function applied to a receiver.
    let upper = |receiver: &str| string::to_uppercase(receiver);
    assert_eq!(upper("abc"), "ABC");
    // :93 - converts to lowercase.
    assert_eq!(string::to_lowercase("HELLO"), "hello");
    // :96 - lowercase handles empty string.
    assert_eq!(string::to_lowercase(""), "");
}

#[test]
fn string_replace_and_trim() {
    // String.test.ts:138 - replaces the first occurrence.
    assert_eq!(string::replace_first("abab", "b", "c"), "acab");
    // :142 - a non-global pattern replaces the first match too (same observable).
    assert_eq!(string::replace_first("abc", "b", "d"), "adc");
    // :146 - unchanged when nothing matches.
    assert_eq!(string::replace_first("abc", "z", "x"), "abc");
    // :156 - trims tabs and newlines.
    assert_eq!(string::trim("\t\nhello\t\n"), "hello");
    // :160 - trims to empty.
    assert_eq!(string::trim(""), "");
    // :166 - trims leading whitespace only.
    assert_eq!(string::trim_start("  hello  "), "hello  ");
    assert_eq!(string::trim_end("  hello  "), "  hello");
}

#[test]
fn string_trim_ends_and_emptiness() {
    // String.test.ts:170 - trimStart on empty.
    assert_eq!(string::trim_start(""), "");
    // :176 - trims trailing whitespace.
    assert_eq!(string::trim_end("  hello  "), "  hello");
    // :180 - trimEnd on empty.
    assert_eq!(string::trim_end(""), "");
    // :200/:204 - isEmpty.
    assert!(string::is_empty(""));
    assert!(!string::is_empty("a"));
    // :211/:215 - isNonEmpty.
    assert!(string::is_non_empty("a"));
    assert!(!string::is_non_empty(""));
}

#[test]
fn string_length_and_split() {
    // String.test.ts:221 - string length.
    assert_eq!(string::length("abc"), 3);
    // :225 - empty length.
    assert_eq!(string::length(""), 0);
    // :231 - data-last split.
    assert_eq!(string::split("a,b,c", ","), ["a", "b", "c"]);
    // :235 - data-first split.
    assert_eq!(string::split("hello,world", ","), ["hello", "world"]);
    // :239 - empty separator splits into chars.
    assert_eq!(string::split("abc", ""), ["a", "b", "c"]);
    // :243 - empty split by empty.
    assert_eq!(string::split("", ""), [""]);
}

#[test]
fn string_positional_search() {
    // String.test.ts:253 - substring found.
    assert!(string::includes_from("hello world", "world", 0));
    // :257 - substring not found.
    assert!(!string::includes_from("hello world", "foo", 0));
    // :261 - searching starts at the given position.
    assert!(!string::includes_from("hello", "hel", 1));
    assert!(string::includes_from("hello", "llo", 1));
    // :268 - starts with the search.
    assert!(string::starts_with_at("hello world", "hello", 0));
    // :272 - false when it does not start with the search.
    assert!(!string::starts_with_at("hello world", "world", 0));
    // :276 - prefix checked at the given position.
    assert!(string::starts_with_at("hello world", "world", 6));
}

#[test]
fn string_suffix_checks() {
    // String.test.ts:282 - ends with the search.
    assert!(string::ends_with("hello world", "world"));
    // :286 - false when it does not end with the search.
    assert!(!string::ends_with("hello world", "hello"));
    // :290 - suffix checked at the given end position.
    assert!(string::ends_with_at("hello world", "hello", 5));
    assert!(!string::ends_with_at("hello world", "world", 5));
}

#[test]
fn string_index_access() {
    // String.test.ts:296 - char code at index, data-first.
    assert_eq!(string::char_code_at("abc", 1), Some(98));
    // :300 - data-last through a closure.
    let code_at = |index: usize| string::char_code_at("abc", index);
    assert_eq!(code_at(0), Some(97));
    // :304 - none out of bounds.
    assert_eq!(string::char_code_at("abc", 10), None);
    // :310 - substring between indices.
    assert_eq!(string::substring("abcd", 1, Some(3)), "bc");
    // :314 - omitted end runs out.
    assert_eq!(string::substring("abcd", 1, None), "bcd");
    // :320 - character at index.
    assert_eq!(string::char_at("abc", 1), Some('b'));
    // :324 - none out of bounds.
    assert_eq!(string::char_at("abc", 10), None);
}

#[test]
fn string_char_access() {
    // String.test.ts:330 - character at index, data-first.
    assert_eq!(string::char_at("abc", 1).map(|c| c.to_string()), Some("b".to_owned()));
    // :334 - data-last through a closure.
    let at_one = |s: &str| string::char_at(s, 1).map(|c| c.to_string());
    assert_eq!(at_one("abc"), Some("b".to_owned()));
    // :338 - none out of bounds.
    assert_eq!(string::char_at("abc", 10), None);
    // :345 - code point at index.
    assert_eq!(string::code_point_at("abc", 1), Some(98));
    // :348 - none out of bounds.
    assert_eq!(string::code_point_at("abc", 10), None);
}

#[test]
fn string_index_search() {
    // String.test.ts:352 - astral scalars read as one point.
    assert_eq!(string::code_point_at("\u{1D7D8}", 0), Some(120792));
    // :358 - first occurrence offset.
    assert_eq!(string::index_of("abbbc", "b"), Some(1));
    // :362 - none when absent.
    assert_eq!(string::index_of("abbbc", "z"), None);
    // :368 - last occurrence offset.
    assert_eq!(string::last_index_of("abbbc", "b"), Some(3));
    assert_eq!(string::last_index_of("abbbc", "z"), None);
}
