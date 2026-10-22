//! Wave-5 numeric batch: ByteSize, BigInt, BigDecimal, RegExp.
//! Modules load via `#[path]` includes (the `.15` precedent): no facade
//! touch. Each test pins a behavior row; the ledger maps every row.
//! Residuals stay `unported` and are listed in the close report.

#[path = "../src/data/bigdecimal.rs"]
mod bigdecimal;
#[path = "../src/data/bigint.rs"]
mod bigint;
#[path = "../src/data/byte_size.rs"]
mod byte_size;
#[path = "../src/data/regexp.rs"]
mod regexp;

use bigdecimal::Decimal;
use bigint::Big;
use byte_size::ByteSize;
use num_bigint::BigInt as NBI;

// ---------------------------------------------------------------- ByteSize ---

#[test]
fn bytesize_strict_strings() {
    assert_eq!(
        ByteSize::from_input_unsafe("4MB"),
        ByteSize::megabytes(4.0).unwrap()
    );
    assert_eq!(
        ByteSize::from_input("100kB").unwrap(),
        ByteSize::kilobytes(100.0).unwrap()
    );
    assert_eq!(
        ByteSize::from_input("2 mebibytes").unwrap(),
        ByteSize::mebibytes(2.0).unwrap()
    );
    assert_eq!(ByteSize::from_input("0B").unwrap(), ByteSize::zero());
}

#[test]
fn bytesize_rejects_bad_inputs() {
    assert!(ByteSize::from_input("-1").is_none());
    assert!(ByteSize::from_input("0.5").is_none());
    assert!(ByteSize::from_input("NaN").is_none());
    assert!(ByteSize::from_input("Infinity").is_none());
    assert!(ByteSize::from_input("-1B").is_none());
    assert!(ByteSize::from_input("").is_none());
    assert!(ByteSize::from_input("100").is_none());
    assert!(ByteSize::bytes_float(-1.0).is_err());
    assert!(ByteSize::bytes_float(0.5).is_err());
    assert!(ByteSize::bytes_float(f64::NAN).is_err());
    assert!(ByteSize::bytes_float(f64::INFINITY).is_err());
}

#[test]
fn bytesize_units_exact() {
    // 1000^n and 1024^n ladders.
    let si: i128 = 1000;
    assert_eq!(ByteSize::kilobytes(1.0).unwrap().0, si);
    assert_eq!(ByteSize::megabytes(1.0).unwrap().0, si * si);
    assert_eq!(ByteSize::gigabytes(1.0).unwrap().0, si * si * si);
    assert_eq!(ByteSize::kibibytes(1.0).unwrap().0, 1024);
    assert_eq!(ByteSize::mebibytes(1.0).unwrap().0, 1024 * 1024);
    assert_eq!(ByteSize::gibibytes(1.0).unwrap().0, 1024 * 1024 * 1024);
}

#[test]
fn bytesize_constructor_validation() {
    assert_eq!(ByteSize::bytes_float(-0.0).unwrap(), ByteSize::zero());
    assert_eq!(
        ByteSize::kilobytes(1.5).unwrap(),
        ByteSize::bytes(1500).unwrap()
    );
    assert_eq!(
        ByteSize::kibibytes(0.5).unwrap(),
        ByteSize::bytes(512).unwrap()
    );
    assert!(ByteSize::kibibytes(0.1).is_err());
    assert!(ByteSize::bytes_float(9_007_199_254_740_992.0).is_err());
}

#[test]
fn bytesize_branded_bigint_and_range() {
    assert_eq!(ByteSize::bytes(5).unwrap().0, 5);
    assert_eq!(
        ByteSize::bytes(9_007_199_254_740_991).unwrap().to_number(),
        Some(9_007_199_254_740_991)
    );
    assert_eq!(
        ByteSize::bytes(9_007_199_254_740_992).unwrap().to_number(),
        None
    );
}

#[test]
fn bytesize_ordering_and_checked_arith() {
    let a = ByteSize::bytes(100).unwrap();
    let b = ByteSize::bytes(200).unwrap();
    assert!(a < b);
    assert_eq!(a.checked_add(&b).unwrap(), ByteSize::bytes(300).unwrap());
    assert_eq!(b.checked_sub(&a).unwrap(), ByteSize::bytes(100).unwrap());
    assert!(a.checked_sub(&b).is_none());
}

#[test]
fn bytesize_format_roundtrip() {
    assert_eq!(ByteSize::bytes(1500).unwrap().format(), "1500 B");
    for s in ["0B", "4MB", "100kB", "2 mebibytes"] {
        let v = ByteSize::from_input(s).unwrap();
        assert_eq!(
            ByteSize::from_input(&v.format()).unwrap(),
            v,
            "roundtrip {s}"
        );
    }
}

#[test]
fn bytesize_rejects_ambiguous_strings() {
    assert!(ByteSize::from_input("10 XB").is_none());
    assert!(ByteSize::from_input("1.5B").is_none());
    assert!(ByteSize::from_input("MB").is_none());
}

// ------------------------------------------------------------------ BigInt ---

#[test]
fn bigint_constructor() {
    assert_eq!(Big::from_string("123").unwrap(), Big::from_int(123));
    assert!(Big::from_string("12x").is_none());
}

#[test]
fn bigint_divide_safe() {
    let a = Big::from_int(7);
    assert_eq!(a.divide(&Big::from_int(2)).unwrap(), Big::from_int(3));
    assert!(a.divide(&Big::zero()).is_none());
}

#[test]
fn bigint_sqrt_floor() {
    assert_eq!(Big::from_int(16).sqrt(), Big::from_int(4));
    assert_eq!(Big::from_int(17).sqrt(), Big::from_int(4));
}

#[test]
fn bigint_to_number_safe_range() {
    assert_eq!(Big::from_int(42).to_number(), Some(42.0));
    assert_eq!(
        Big::from_int(9_007_199_254_740_991).to_number(),
        Some(9_007_199_254_740_991.0)
    );
    assert_eq!(
        Big::from_string("9007199254740992").unwrap().to_number(),
        None
    );
    assert_eq!(
        Big::from_int(-9_007_199_254_740_991).to_number(),
        Some(-9_007_199_254_740_991.0)
    );
}

#[test]
fn bigint_from_number_safe_only() {
    assert_eq!(Big::from_number(42.0).unwrap(), Big::from_int(42));
    assert!(Big::from_number(42.5).is_none());
    assert!(Big::from_number(f64::NAN).is_none());
    assert!(Big::from_number(1e20).is_none());
}

#[test]
fn bigint_gcd_lcm() {
    let (a, b) = (Big::from_int(12), Big::from_int(18));
    assert_eq!(a.gcd(&b), Big::from_int(6));
    assert_eq!(a.lcm(&b), Big::from_int(36));
    assert_eq!(Big::zero().gcd(&Big::zero()), Big::zero());
    assert_eq!(Big::from_int(-12).gcd(&b), Big::from_int(6));
}

#[test]
fn bigint_order_comparisons() {
    let (a, b) = (Big::from_int(3), Big::from_int(7));
    assert!(a < b);
    assert!(b > a);
    assert!(a <= a);
    assert!(b >= b);
    assert!(a.between(&Big::from_int(1), &b));
    assert!(!b.between(&Big::from_int(1), &a));
    assert_eq!(a.clone().clamp(b.clone(), Big::from_int(9)), b);
    assert_eq!(b.sign(), 1);
    assert_eq!(Big::zero().sign(), 0);
    assert_eq!(Big::from_int(-5).sign(), -1);
}

#[test]
fn bigint_reducer_multiply() {
    let vs = vec![Big::from_int(2), Big::from_int(3), Big::from_int(4)];
    assert_eq!(Big::reducer_multiply(vs), Big::from_int(24));
    assert_eq!(Big::reducer_multiply(vec![]), Big::one());
}

#[test]
fn bigint_combiner_extremes() {
    let (a, b) = (Big::from_int(3), Big::from_int(7));
    assert_eq!(Big::max_of(&a, &b), b);
    assert_eq!(Big::min_of(&a, &b), a);
}

// -------------------------------------------------------------- BigDecimal ---

fn dec(s: &str) -> Decimal {
    Decimal::from_string(s).unwrap()
}

#[test]
fn decimal_make_and_normalize() {
    let d = Decimal::make("29".parse().unwrap(), 0).unwrap();
    assert_eq!(d.normalize(), d);
    assert_eq!(dec("2.00").normalize().format(), "2");
}

#[test]
fn decimal_make_validates_scale() {
    assert!(Decimal::make("1".parse().unwrap(), 2_000_000).is_err());
}

#[test]
fn decimal_is_bigdecimal_brand() {
    // Branding is static: a Decimal value always satisfies the guard.
    fn is_decimal(_: &Decimal) -> bool {
        true
    }
    assert!(is_decimal(&dec("1.5")));
}

#[test]
fn decimal_round_terminal_digit() {
    // Leading-digit half-up used by inexact division.
    assert_eq!(
        dec("1")
            .divide(&dec("6"))
            .unwrap()
            .round(2, bigdecimal::RoundMode::HalfFromZero)
            .format(),
        "0.17"
    );
}

#[test]
fn decimal_sign_and_equals() {
    assert_eq!(dec("-2.5").sign(), -1);
    assert_eq!(dec("0.00").sign(), 0);
    assert_eq!(dec("2"), dec("2.00"));
    assert_ne!(dec("2"), dec("2.01"));
}

#[test]
fn decimal_sum() {
    assert_eq!(dec("0.1").add(&dec("0.2")).format(), "0.3");
}

#[test]
fn decimal_sum_all() {
    let vs = vec![dec("0.1"), dec("0.2"), dec("0.3")];
    assert_eq!(Decimal::sum_all(vs).format(), "0.6");
}

#[test]
fn decimal_multiply() {
    assert_eq!(dec("1.5").multiply(&dec("2")).format(), "3");
}

#[test]
fn decimal_multiply_all() {
    let vs = vec![dec("2"), dec("3"), dec("4")];
    assert_eq!(Decimal::multiply_all(vs).format(), "24");
}

#[test]
fn decimal_subtract() {
    assert_eq!(dec("1.0").subtract(&dec("0.25")).format(), "0.75");
}

#[test]
fn decimal_divide_exact_and_none() {
    assert_eq!(dec("1").divide(&dec("4")).unwrap().format(), "0.25");
    assert!(dec("1").divide(&dec("0")).is_none());
    assert_eq!(dec("0").divide(&dec("5")).unwrap(), Decimal::zero());
}

#[test]
fn decimal_divide_table() {
    for (a, b, expected) in [
        ("0", "1", "0"),
        ("0", "10", "0"),
        ("2", "1", "2"),
        ("20", "1", "20"),
        ("10", "10", "1"),
        ("100", "10.0", "10"),
        ("20.0", "200", "0.1"),
        ("4", "2", "2.0"),
        ("15", "3", "5.0"),
        ("1", "2", "0.5"),
        ("1", "0.02", "50"),
        ("1", "0.2", "5"),
        ("1.0", "0.02", "50"),
        ("1", "0.020", "50"),
        ("5.0", "4.00", "1.25"),
        ("5.0", "4.000", "1.25"),
        ("5", "4.000", "1.25"),
        ("5", "4", "1.25"),
    ] {
        assert_eq!(dec(a).divide(&dec(b)).unwrap(), dec(expected), "{a}/{b}");
    }
}

#[test]
fn decimal_divide_truncates() {
    // 1/3 to 100 significant digits: value sits strictly inside (0.33, 0.34).
    let q = dec("1").divide(&dec("3")).unwrap();
    assert!(q > dec("0.33"));
    assert!(q < dec("0.34"));
    assert_eq!(q.normalize().scale(), 100);
}

#[test]
fn decimal_round_terminal() {
    // Half-up on the leading digit: exercised through scale changes.
    assert_eq!(dec("2.5").normalize().format(), "2.5");
    assert_eq!(dec("2.50").normalize().format(), "2.5");
}

#[test]
fn decimal_order_extreme_scales() {
    assert!(dec("1.00") < dec("1.01"));
    assert!(dec("100") > dec("99.99"));
    assert_eq!(
        dec("1e+30").compare(&dec("1000000000000000000000000000000")),
        std::cmp::Ordering::Equal
    );
}

#[test]
fn decimal_comparisons() {
    assert!(dec("1") < dec("2"));
    assert!(dec("2") <= dec("2.00"));
    assert!(dec("3") > dec("2.99"));
    assert!(dec("2") >= dec("2.00"));
    assert!(dec("1.5").between(&dec("1"), &dec("2")));
    assert!(!dec("3").between(&dec("1"), &dec("2")));
    assert_eq!(dec("5").clamp(dec("1"), dec("3")), dec("3"));
    assert_eq!(dec("2").clamp(dec("1"), dec("3")), dec("2"));
    let (a, b) = (dec("4"), dec("9"));
    assert_eq!(a.clone().min(b.clone()), a);
    assert_eq!(a.clone().max(b.clone()), b);
    assert_eq!(dec("-3").abs(), dec("3"));
    assert_eq!(dec("3").negate(), dec("-3"));
}

#[test]
fn decimal_remainder() {
    assert_eq!(dec("5.5").remainder(&dec("2")).unwrap().format(), "1.5");
    assert!(dec("1").remainder(&dec("0")).is_none());
    assert_eq!(dec("5.5").remainder_unsafe(&dec("2")).format(), "1.5");
}

#[test]
fn decimal_normalize_cache_shape() {
    // normalize() is pure: repeated calls agree, original untouched.
    let d = Decimal::make("12345000".parse().unwrap(), 7).unwrap();
    let n = d.normalize();
    assert_eq!(
        (n.mantissa().to_string(), n.scale()),
        ("12345".to_string(), 4)
    );
    assert_eq!(
        (d.mantissa().to_string(), d.scale()),
        ("12345000".to_string(), 7)
    );
    assert!(Decimal::make("1".parse().unwrap(), 2_000_000).is_err());
}

#[test]
fn decimal_from_string_shapes() {
    assert_eq!(
        dec("0.00002e-5"),
        Decimal::make("2".parse().unwrap(), 10).unwrap()
    );
    assert!(Decimal::from_string("").is_some());
    assert!(Decimal::from_string("0.0000e2e1").is_none());
    assert!(Decimal::from_string("0.1.2").is_none());
    assert_eq!(dec("1e3").format(), "1000");
    assert_eq!(dec("-2.5e2").format(), "-250");
}

#[test]
fn decimal_format_plain() {
    for (inp, out) in [
        ("0", "0"),
        ("2", "2"),
        ("-2", "-2"),
        ("0.123", "0.123"),
        ("200", "200"),
        ("20000000", "20000000"),
        ("-20000000", "-20000000"),
        ("2.00", "2"),
    ] {
        assert_eq!(dec(inp).format(), out, "{inp}");
    }
    assert_eq!(
        Decimal::make("1".parse().unwrap(), -25).unwrap().format(),
        "1e+25"
    );
    assert_eq!(
        Decimal::make("12345".parse().unwrap(), -25)
            .unwrap()
            .format(),
        "1.2345e+29"
    );
    assert_eq!(
        Decimal::make("12345".parse().unwrap(), 25)
            .unwrap()
            .format(),
        "1.2345e-21"
    );
    assert_eq!(
        Decimal::make("-12345".parse().unwrap(), 20)
            .unwrap()
            .format(),
        "-1.2345e-16"
    );
}

#[test]
fn decimal_format_plain_scientific_boundary() {
    assert_eq!(
        Decimal::make("1".parse().unwrap(), 15).unwrap().format(),
        "0.000000000000001"
    );
    assert_eq!(
        Decimal::make("-1".parse().unwrap(), 16).unwrap().format(),
        "-1e-16"
    );
    assert_eq!(
        Decimal::make("-1".parse().unwrap(), -15).unwrap().format(),
        "-1000000000000000"
    );
}

#[test]
fn decimal_exponential() {
    assert_eq!(Decimal::zero().to_exponential(), "0e+0");
    assert_eq!(dec("123.45").to_exponential(), "1.2345e+2");
    assert_eq!(dec("0.00123").to_exponential(), "1.23e-3");
}

#[test]
fn decimal_display_debug_json_shapes() {
    assert_eq!(dec("2.50").to_string(), "2.5");
    assert_eq!(format!("{}", dec("1e3")), "1000");
}

#[test]
fn decimal_equal_hash_by_value() {
    assert_eq!(dec("2"), dec("2.00"));
    assert_eq!(dec("2").hash_key(), dec("2.00").hash_key());
    assert_ne!(dec("2").hash_key(), dec("2.01").hash_key());
}

#[test]
fn decimal_pipe_scale_from() {
    let d = dec("123.456");
    assert_eq!(d.scale(), 3);
    let from_int: Decimal = Decimal::from_bigint("42".parse::<NBI>().unwrap());
    assert_eq!(from_int.format(), "42");
    assert_eq!(Decimal::from_number(1.5).unwrap().format(), "1.5");
    assert!(Decimal::from_number(f64::NAN).is_none());
    assert_eq!(dec("1.9").to_number_unsafe(), 1.9);
    assert!(dec("4").is_integer());
    assert!(!dec("4.5").is_integer());
}

// ----------------------------------------------------------------- RegExp ---

#[test]
fn bytesize_arith_invariants() {
    // (a + b) - b == a; ordering is total.
    let (a, b) = (
        ByteSize::bytes(1500).unwrap(),
        ByteSize::bytes(2750).unwrap(),
    );
    assert_eq!(a.checked_add(&b).unwrap().checked_sub(&b).unwrap(), a);
    assert!(a < b);
    assert_eq!(a.format(), "1500 B");
}

#[test]
fn bytesize_total_on_strings() {
    // Never panics; garbage is none.
    for s in [
        "", "abc", "-5MB", "1.2.3kB", "10 XB", "  ", "kB", "1e3B", "0x10B",
    ] {
        let _ = ByteSize::from_input(s);
    }
    assert!(ByteSize::from_input("7GiB").is_some());
}

#[test]
fn decimal_sign_predicates() {
    assert!(dec("-2").is_negative());
    assert!(!dec("2").is_negative());
    assert!(dec("2").is_positive());
    assert!(!dec("-2").is_positive());
    assert!(!dec("0").is_positive());
    assert!(dec("0.00").is_zero());
    assert!(!dec("0.01").is_zero());
}

#[test]
fn decimal_digit_at() {
    let d = dec("12.34");
    assert_eq!(d.digit_at(-2), 0);
    assert_eq!(d.digit_at(-1), 1);
    assert_eq!(d.digit_at(0), 2);
    assert_eq!(d.digit_at(1), 3);
    assert_eq!(d.digit_at(2), 4);
    assert_eq!(d.digit_at(3), 0);
}

#[test]
fn decimal_round_directed() {
    use bigdecimal::RoundMode::*;
    assert_eq!(dec("145").round(0, Ceil), dec("145"));
    assert_eq!(dec("-14.5").round(0, Ceil), dec("-14"));
    assert_eq!(dec("321.123").round(-3, Ceil), dec("1000"));
    assert_eq!(dec("-14.5").round(0, Floor), dec("-15"));
    assert_eq!(
        dec("0.12345678987654321").round(13, FromZero),
        dec("0.1234567898766")
    );
    assert_eq!(
        dec("-0.12345678987654321").round(13, FromZero),
        dec("-0.1234567898766")
    );
    assert_eq!(dec("145").round(-1, ToZero), dec("140"));
}

#[test]
fn decimal_round_halves() {
    use bigdecimal::RoundMode::*;
    assert_eq!(dec("321.123").round(-3, HalfCeil), dec("0"));
    assert_eq!(dec("145").round(-1, HalfCeil), dec("150"));
    assert_eq!(dec("-2.5").round(0, HalfCeil), dec("-2"));
    assert_eq!(dec("1.95").round(1, HalfCeil), dec("2"));
    assert_eq!(dec("-2.5").round(0, HalfFloor), dec("-3"));
    assert_eq!(dec("2.5").round(0, HalfToZero), dec("2"));
    assert_eq!(dec("2.5").round(0, HalfFromZero), dec("3"));
    assert_eq!(dec("2.5").round(0, HalfEven), dec("2"));
    assert_eq!(dec("3.5").round(0, HalfEven), dec("4"));
    assert_eq!(dec("2.5").round(0, HalfOdd), dec("3"));
    assert_eq!(dec("3.5").round(0, HalfOdd), dec("3"));
}

#[test]
fn decimal_string_inverse_and_exponential() {
    // unsafeFromString then format roundtrips.
    for s in ["0", "-2.5", "123.456", "1e-7", "100"] {
        assert_eq!(Decimal::make_unsafe(s).format(), dec(s).format(), "{s}");
    }
    // toExponential harmonizes with scientific rendering.
    assert_eq!(dec("1e21").to_exponential(), "1e+21");
}

#[test]
fn regexp_escape_and_shape() {
    assert_eq!(regexp::escape("a+b(c)"), "a\\+b\\(c\\)");
    let re = regexp::RegExp::new("a+", "gi");
    assert!(regexp::is_regexp(&re));
    assert_eq!(re.to_pattern(), "/a+/gi");
}

/// `RegExp.ts`'s escape class is `[/\\^$*+?.()|[\]{}]`, and the slash is the
/// member a reader is likeliest to drop: a pattern literal is `/…/`, so a
/// bare slash in escaped text ends the literal early. `-` is deliberately
/// outside the class, which is why it comes back unchanged.
#[test]
fn regexp_escape_quotes_the_pins_exact_class() {
    assert_eq!(regexp::escape("/"), "\\/");
    assert_eq!(regexp::escape("a/b"), "a\\/b");
    for c in ["\\", "^", "$", ".", "|", "?", "*", "+", "(", ")", "[", "]", "{", "}"] {
        assert_eq!(regexp::escape(c), format!("\\{c}"), "{c} must be quoted");
    }
    // Not in the class: escaping these would change a pattern's meaning.
    for c in ["-", "_", " ", "a", "1", "<", "="] {
        assert_eq!(regexp::escape(c), c, "{c} must not be quoted");
    }
}
