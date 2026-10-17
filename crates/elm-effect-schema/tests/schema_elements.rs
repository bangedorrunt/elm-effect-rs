//! Slice 7: tuple derivations that rewrite the element list.
//!
//! The pin's `Tuple.mapElements` takes an element transformer, and each of its
//! helpers (`Tuple.appendElement`, `Tuple.evolve`, `Tuple.renameIndices`, ...)
//! builds one. These tests assert the derived *element list*, which is what the
//! pin's rows compare, so the helper reads the elements back through
//! `Schema::elements`.
//! Binds Schema.test.ts rows 8982-9038.

use elm_effect_schema::{
    Schema, Unknown, element_append, element_append_all, element_evolve, element_make_nullable,
    element_make_optional_key, element_map, element_omit, element_pick, element_rename_indices,
};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn array(items: Vec<Unknown>) -> Unknown {
    Unknown::Array(items)
}

fn str_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn bool_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::boolean())
}

/// The derived shape in the pin's terms: the code of each element, in position
/// order.
fn elements_of(schema: &Schema<Unknown>) -> Vec<String> {
    schema
        .elements()
        .expect("a derived tuple is still a tuple shape")
        .iter()
        .map(|element| element.to_code())
        .collect()
}

fn codes(codes: &[&str]) -> Vec<String> {
    codes.iter().map(|code| code.to_string()).collect()
}

fn pair_of_numbers() -> Schema<Unknown> {
    Schema::tuple_of(vec![str_schema(), number_schema()])
}

fn three_elements() -> Schema<Unknown> {
    Schema::tuple_of(vec![str_schema(), number_schema(), bool_schema()])
}

// -------------------------------------------------------- Tuple.mapElements ---

/// Schema.test.ts:8982 `appendElement`: one position added at the end.
#[test]
fn append_element_adds_a_trailing_position() {
    let schema = Schema::tuple_of(vec![str_schema()]).map_elements(element_append(number_schema()));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.String", "Schema.Number"])
    );
}

/// The point of rebuilding the tuple rather than only its element list: the
/// decoder follows the new arity. A row that compared `elements` alone would
/// pass with the old decoder still wired in.
#[test]
fn append_element_widens_the_decoder() {
    let schema = Schema::tuple_of(vec![str_schema()]).map_elements(element_append(number_schema()));

    assert_eq!(
        schema
            .decode(array(vec![text("a"), Unknown::Number(1.0)]))
            .unwrap(),
        array(vec![text("a"), Unknown::Number(1.0)])
    );
    assert!(
        schema.decode(array(vec![text("a")])).is_err(),
        "the widened tuple is two positions, not one"
    );
}

/// Schema.test.ts:8988 `appendElements`: several positions added at once.
#[test]
fn append_elements_adds_every_listed_position() {
    let schema = Schema::tuple_of(vec![str_schema()])
        .map_elements(element_append_all(vec![number_schema(), bool_schema()]));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.String", "Schema.Number", "Schema.Boolean"])
    );
}

/// Schema.test.ts:9000 `omit`: the listed positions drop out, the rest keep
/// their order.
#[test]
fn omit_drops_the_listed_position() {
    let schema = three_elements().map_elements(element_omit(vec![1]));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.String", "Schema.Boolean"])
    );
}

/// Neither `pick` row pins the reading order: the pin's `Tuple.pick` maps the
/// given indices in the order given (`indices.map(i => self[i])`), so a
/// descending list comes back descending.
#[test]
fn pick_keeps_the_order_it_was_given() {
    let schema = three_elements().map_elements(element_pick(vec![2, 0]));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.Boolean", "Schema.String"])
    );
}

/// Schema.test.ts:9007 `evolve` `readonly [string] -> readonly [string?]`.
#[test]
fn evolve_makes_a_trailing_position_optional() {
    let schema = Schema::tuple_of(vec![str_schema()])
        .map_elements(element_evolve(vec![Some(element_make_optional_key)]));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.optionalKey(Schema.String)"])
    );
}

/// Schema.test.ts:9013 `evolve` `readonly [string, number] -> readonly
/// [string, number?]`: the `undefined` entry leaves that position alone.
#[test]
fn evolve_touches_only_the_listed_position() {
    let schema =
        pair_of_numbers().map_elements(element_evolve(vec![None, Some(element_make_optional_key)]));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.String", "Schema.optionalKey(Schema.Number)"])
    );
}

/// Schema.test.ts:9023 `renameIndices` `partial index mapping`: `order[0] = 1`
/// names the old position that moves to the front; positions the list does not
/// reach keep their own index.
#[test]
fn rename_indices_rewrites_the_mapped_prefix() {
    let schema = three_elements().map_elements(element_rename_indices(vec![1, 0]));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.Number", "Schema.String", "Schema.Boolean"])
    );
}

/// Schema.test.ts:9030 `full index mapping`: the whole list reverses.
#[test]
fn rename_indices_permutes_every_position() {
    let schema = three_elements().map_elements(element_rename_indices(vec![2, 1, 0]));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.Boolean", "Schema.Number", "Schema.String"])
    );
}

/// Both `renameIndices` rows are involutions, so they read the same under
/// either direction. The pin reads the source at each destination
/// (`self.map((e, i) => self[mapping[i]])`), and an asymmetric mapping is what
/// tells the two apart: the inverse reading would answer
/// `[Number, Boolean, String]` here.
#[test]
fn rename_indices_reads_each_destination_from_its_source() {
    let schema = three_elements().map_elements(element_rename_indices(vec![2, 0, 1]));

    assert_eq!(
        elements_of(&schema),
        codes(&["Schema.Boolean", "Schema.String", "Schema.Number"])
    );
}

/// Schema.test.ts:9038 `NullOr`: every element becomes null-or, the way the
/// field twin does.
#[test]
fn null_or_wraps_every_element() {
    let schema = pair_of_numbers().map_elements(element_map(element_make_nullable));

    assert_eq!(
        elements_of(&schema),
        codes(&[
            "Schema.optional(Schema.String)",
            "Schema.optional(Schema.Number)"
        ])
    );
}

// ----------------------------------------------------------------- guards ---

/// The pin refuses a tuple with a required position after an optional one
/// (`ts(1257)`: the later values have no position to occupy). The port has no
/// type checker at the derivation site, so the shape refuses when it is built.
#[test]
#[should_panic(expected = "A required element cannot follow an optional element")]
fn a_required_position_after_an_optional_one_refuses() {
    let _ = Schema::tuple_of(vec![
        Schema::optional_key_shape(str_schema()),
        number_schema(),
    ]);
}

/// `mapElements` is a tuple derivation, so a shape with no element list refuses
/// rather than inventing one.
#[test]
#[should_panic(expected = "mapElements needs a tuple shape")]
fn map_elements_refuses_a_shape_without_elements() {
    let _ = str_schema().map_elements(element_append(number_schema()));
}
