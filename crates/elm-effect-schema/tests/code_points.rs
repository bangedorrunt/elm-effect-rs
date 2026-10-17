use elm_effect_schema::{Schema, Unknown};

fn checked(name: &str, args: Vec<&str>) -> Schema<Unknown> {
    Schema::checked(Schema::as_unknown(Schema::string()), name, args, vec![])
}

fn cp_count(s: &str) -> usize {
    s.chars().count()
}

#[test]
fn code_points_min_max_boundaries() {
    let cases = [
        "", "a", "abc", "a😀b", "\r\n",
    ];
    for bound in 0..=8usize {
        let min = checked("isMinCodePoints", vec![&bound.to_string()]);
        let max = checked("isMaxCodePoints", vec![&bound.to_string()]);
        let exact = checked(
            "isBetweenCodePoints",
            vec![&bound.to_string(), &bound.to_string()],
        );
        for value in cases {
            let count = cp_count(value);
            assert_eq!(min.decode(Unknown::Text(value.into())).is_ok(), count >= bound);
            assert_eq!(max.decode(Unknown::Text(value.into())).is_ok(), count <= bound);
            assert_eq!(exact.decode(Unknown::Text(value.into())).is_ok(), count == bound);
        }
    }
}

#[test]
fn code_points_inclusive_and_inverted() {
    let between = checked("isBetweenCodePoints", vec!["1", "3"]);
    let inverted = checked("isBetweenCodePoints", vec!["3", "1"]);
    let cases = ["", "a", "abc", "\r\n"];
    for value in cases {
        let count = cp_count(value);
        assert_eq!(between.decode(Unknown::Text(value.into())).is_ok(), count >= 1 && count <= 3);
        assert!(inverted.decode(Unknown::Text(value.into())).is_err());
    }
}

#[test]
fn code_points_round_down_and_clamp() {
    let min = checked("isMinCodePoints", vec!["1.9"]);
    let max = checked("isMaxCodePoints", vec!["1.9"]);
    let between = checked("isBetweenCodePoints", vec!["1.9", "2.9"]);
    assert!(min.decode(Unknown::Text("😀".into())).is_ok());
    assert!(!max.decode(Unknown::Text("😀a".into())).is_ok());
    assert!(max.decode(Unknown::Text("😀".into())).is_ok());
    assert!(between.decode(Unknown::Text("😀a".into())).is_ok());
    assert!(checked("isMinCodePoints", vec!["-1"]).decode(Unknown::Text("".into())).is_ok());
    assert!(checked("isMaxCodePoints", vec!["-1"]).decode(Unknown::Text("".into())).is_ok());
    assert!(checked("isMaxCodePoints", vec!["-1"]).decode(Unknown::Text("a".into())).is_err());
    assert!(checked("isBetweenCodePoints", vec!["-2", "-1"]).decode(Unknown::Text("".into())).is_ok());
}

#[test]
fn code_points_messages() {
    let min = checked("isMinCodePoints", vec!["1"]);
    let max = checked("isMaxCodePoints", vec!["1"]);
    let between = checked("isBetweenCodePoints", vec!["1", "2"]);
    assert!(min.decode(Unknown::Text("".into())).is_err());
    assert!(max.decode(Unknown::Text("😀a".into())).is_err());
    assert!(between.decode(Unknown::Text("😀ab".into())).is_err());
    assert!(between.decode(Unknown::Text("😀a".into())).is_ok());
}

#[test]
fn code_points_custom_annotations() {
    let schema = checked("isMaxCodePoints", vec!["1"]);
    // attaching expected annotation does not break decoding validity
    let _ = schema.decode(Unknown::Text("a".into()));
}
