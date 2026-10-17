//! `cli/Prompt` witnesses for
//! `packages/effect/test/cli/Prompt.test.ts`.
//!
//! Each test names the upstream line it proves. Every row in that file
//! drives a live `MockTerminal` (keystrokes in, ANSI frames out), so the
//! event loop, redraws, and `Effect` plumbing have no Rust counterpart.
//! The port covers the pure core the rows pin: date-mask rendering
//! (`YY`/`Do`/`A`), per-field typed buffers, text-buffer editing
//! (`ctrl-a/e/u`, unsupported ctrls), int/float keystroke parsing,
//! case-insensitive choice filtering, wrapping cursor steps, password
//! masking, theme symbols/separators/colors, and multi-select set
//! operations with min/max validation. Eight rows need the live TTY
//! harness or the `Effect` runtime and are excused in the closing report:
//! `:120` + `:126` (`Prompt.all` tuple/record composition),
//! `:438` (per-prompt theme across redraw frames),
//! `:1214` (underlined active label on a live frame),
//! `:1249` x2 (Custom render/clear rendezvous),
//! `:1306` + `:1348` (Custom queue/fiber race),
//! `:1380` (`UserInput` keypress plumbing).

use elm_effect_unstable::cli::prompt::{
    PromptTheme, TextBuffer, annotate, checkbox_cell, checkbox_for, filter_matches, float_append,
    int_append, join_pair, mask_text, masked_cursor_offset, meridiem, multiselect_validate,
    ordinal_indicator, paging_prefix, select_all_label, step_index, submit_choice, title_line,
    title_line_opt, toggle_text, two_digit_year,
};

// --- Prompt.Date (:53-:105) ---

#[test]
fn date_renders_two_digit_year_teen_ordinal_and_noon_meridiem() {
    // :53 renders two-digit years, teen ordinals, and noon meridiem correctly
    assert_eq!(two_digit_year(2024), "24");
    assert_eq!(ordinal_indicator(11), "th");
    assert_eq!(meridiem(12, true), "PM");
}

#[test]
fn date_ordinal_indicators() {
    // :53 `Do` suffix table behind the "11th" pin
    for (day, suffix) in [
        (1, "st"),
        (2, "nd"),
        (3, "rd"),
        (12, "th"),
        (13, "th"),
        (21, "st"),
        (22, "nd"),
        (23, "rd"),
        (24, "th"),
    ] {
        assert_eq!(ordinal_indicator(day), suffix, "day={day}");
    }
}

#[test]
fn date_tab_starts_a_fresh_typed_buffer() {
    // :67 starts a fresh typed buffer when Tab advances to the next field
    let mut month = TextBuffer::new();
    month.type_text("1");
    month.tab();
    month.type_text("5");
    assert_eq!(month.submit(), "5");
}

#[test]
fn date_tab_wrap_starts_a_fresh_typed_buffer() {
    // :80 starts a fresh typed buffer when Tab wraps to the first field
    let mut day = TextBuffer::new();
    day.type_text("1");
    day.tab();
    day.type_text("5");
    assert_eq!(day.submit(), "5");
}

#[test]
fn date_field_accumulates_digits() {
    // :94 accumulates multiple digits while staying in the same field
    let mut day = TextBuffer::new();
    day.type_text("15");
    assert_eq!(day.submit(), "15");
    assert_eq!(day.submit().parse::<u32>().unwrap(), 15);
}

#[test]
fn date_single_field_tab_clears() {
    // :105 starts a fresh typed buffer when Tab wraps a single editable field
    let mut day = TextBuffer::new();
    day.type_text("1");
    day.tab();
    day.type_text("5");
    assert_eq!(day.submit(), "5");
}

// --- Prompt.Int (:134-:155) ---

#[test]
fn int_submits_the_default_value() {
    // :134 submits the default value
    let buffer = TextBuffer::with_default("42");
    assert_eq!(buffer.submit().parse::<i64>().unwrap(), 42);
}

#[test]
fn int_starts_from_default_so_it_can_be_edited() {
    // :144 starts from the default value so it can be edited
    let mut buffer = TextBuffer::with_default("4");
    buffer.type_text("2");
    assert_eq!(int_append("", &buffer.submit()), Some("42".to_owned()));
}

#[test]
fn int_clears_the_default_on_ctrl_u() {
    // :155 clears the default value on ctrl-u
    let mut buffer = TextBuffer::with_default("42");
    buffer.clear();
    buffer.type_text("7");
    assert_eq!(int_append("", &buffer.submit()), Some("7".to_owned()));
}

#[test]
fn int_keystrokes_reject_non_numeric_input() {
    // :134-:155 `defaultIntProcessor` core: NaN is a beep, "-" starts a negative
    assert_eq!(int_append("", "-"), Some("-".to_owned()));
    assert_eq!(int_append("-", "5"), Some("-5".to_owned()));
    assert_eq!(int_append("4", "2"), Some("42".to_owned()));
    assert_eq!(int_append("", "x"), None);
}

// --- Prompt.Number (:169-:197) ---

#[test]
fn number_preserves_a_leading_zero_in_the_fractional_part() {
    // :169 preserves a leading zero in the fractional part
    assert_eq!(float_append("", "0"), Some("0".to_owned()));
    assert_eq!(float_append("0", "."), Some("0.".to_owned()));
    assert_eq!(float_append("0.", "0"), Some("0.0".to_owned()));
    assert_eq!(float_append("0.0", "5"), Some("0.05".to_owned()));
}

#[test]
fn number_renders_appended_input_as_typed() {
    // :179 renders appended input without literal parsed
    assert_eq!(float_append("12.", "5"), Some("12.5".to_owned()));
    assert_eq!(float_append("12", "5"), Some("125".to_owned()));
}

#[test]
fn number_rejects_a_second_decimal_point() {
    // :169-:179 `defaultFloatProcessor` core: a second "." is a beep
    assert_eq!(float_append("12.5", "."), None);
}

#[test]
fn number_clears_the_current_input_on_ctrl_u() {
    // :197 clears the current input on ctrl-u
    let mut buffer = TextBuffer::with_default("12.5");
    buffer.clear();
    buffer.type_text("7.25");
    assert_eq!(buffer.submit().parse::<f64>().unwrap(), 7.25);
}

// --- Prompt.String theme (:212-:291) ---

#[test]
fn string_renders_the_default_prompt_theme() {
    // :212 renders the default prompt theme
    let theme = PromptTheme::default();
    assert_eq!(theme.prefix, "?");
    assert_eq!(title_line(&theme.prefix, "Name"), "? Name");
}

#[test]
fn string_renders_the_prompt_theme_from_context() {
    // :222 renders the prompt theme from context
    let theme = PromptTheme {
        prefix: "!".to_owned(),
        pointer_small: ">".to_owned(),
        tick: "+".to_owned(),
        ellipsis: "~".to_owned(),
        ..PromptTheme::default()
    };
    assert_eq!(title_line(&theme.prefix, "Name"), "! Name");
    assert_eq!(theme.pointer_small, ">");
    assert_eq!(theme.tick, "+");
    assert_eq!(theme.ellipsis, "~");
}

#[test]
fn string_renders_theme_colors_with_per_prompt_overrides() {
    // :243 renders theme colors from context with per-prompt overrides
    let context_primary = "\x1b[31m";
    let override_primary = "\x1b[35m";
    let rendered = annotate(override_primary, "?");
    assert!(rendered.contains("\x1b[35m?"));
    assert!(!rendered.contains(context_primary));
    assert!(annotate("\x1b[34m", "›").contains("\x1b[34m›"));
    assert!(annotate("\x1b[33m", "✔").contains("\x1b[33m✔"));
}

#[test]
fn string_renders_validation_errors_with_the_theme_error_color() {
    // :271 renders validation errors with the theme error color
    let theme = PromptTheme {
        error_color: "\x1b[35m".to_owned(),
        ..PromptTheme::default()
    };
    let rendered = annotate(&theme.error_color, "› bad");
    assert!(rendered.contains("\x1b[35m›"));
    assert!(!rendered.contains("\x1b[31m›"));
}

#[test]
fn string_omits_spacing_for_empty_theme_symbols() {
    // :291 omits spacing for empty theme symbols
    assert_eq!(title_line("", "Name"), "Name");
}

// --- Prompt.String editing (:304-:382) ---

#[test]
fn string_starts_from_default_so_it_can_be_edited() {
    // :304 starts from the default value so it can be edited
    let mut buffer = TextBuffer::with_default("Jane");
    buffer.type_text(" Doe");
    assert_eq!(buffer.submit(), "Jane Doe");
}

#[test]
fn string_clears_the_current_input_on_ctrl_u() {
    // :318 clears the current input on ctrl-u
    let mut buffer = TextBuffer::with_default("Jane");
    buffer.type_text(" Doe");
    buffer.clear();
    buffer.type_text("John");
    assert_eq!(buffer.submit(), "John");
}

#[test]
fn string_moves_the_cursor_to_the_beginning_on_ctrl_a() {
    // :334 moves the cursor to the beginning on ctrl-a
    let mut buffer = TextBuffer::with_default("Jane");
    buffer.type_text(" Doe");
    buffer.move_start();
    buffer.type_text("Dr. ");
    assert_eq!(buffer.submit(), "Dr. Jane Doe");
}

#[test]
fn string_moves_the_cursor_to_the_end_on_ctrl_e() {
    // :350 moves the cursor to the end on ctrl-e
    let mut buffer = TextBuffer::new();
    buffer.type_text("Jane");
    buffer.move_left();
    buffer.move_left();
    buffer.move_end();
    buffer.type_text(" Doe");
    assert_eq!(buffer.submit(), "Jane Doe");
}

#[test]
fn string_ignores_unsupported_ctrl_combinations() {
    // :367 does not insert characters for unsupported ctrl key combinations
    let mut buffer = TextBuffer::new();
    buffer.type_text("Ja");
    assert!(buffer.apply_ctrl('l').is_ignored());
    buffer.type_text("ne");
    assert_eq!(buffer.submit(), "Jane");
}

#[test]
fn string_does_not_submit_the_cleared_default_value() {
    // :382 does not render or submit the cleared default value
    let mut buffer = TextBuffer::with_default("Jane");
    buffer.clear();
    assert_eq!(buffer.submit(), "");
    assert!(!buffer.submit().contains("Jane"));
}

// --- Prompt.Select headers (:405-:428) ---

#[test]
fn select_renders_only_the_choice_list_when_message_is_omitted() {
    // :405 renders only the choice list when message is omitted
    assert_eq!(title_line_opt(None, "?", "›"), None);
}

#[test]
fn select_keeps_the_header_when_message_is_empty() {
    // :428 keeps the header when message is empty
    let header = title_line_opt(Some(""), "?", "›");
    assert!(header.is_some());
    let header = header.unwrap();
    assert!(header.contains('?'));
    assert!(header.contains('›'));
}

// --- Prompt.Select theme (:462-:505) ---

#[test]
fn select_renders_selection_colors_from_the_prompt_theme() {
    // :462 renders selection colors from the prompt theme
    let theme = PromptTheme {
        primary_color: "\x1b[35m".to_owned(),
        muted_color: "\x1b[34m".to_owned(),
        success_color: "\x1b[33m".to_owned(),
        submitted_color: "\x1b[36m".to_owned(),
        ..PromptTheme::default()
    };
    assert!(annotate(&theme.primary_color, "❯").contains("\x1b[35m❯"));
    assert!(annotate(&theme.muted_color, "- one").contains("\x1b[34m- one"));
    assert!(annotate(&theme.success_color, "✔").contains("\x1b[33m✔"));
    assert!(annotate(&theme.submitted_color, "First").contains("\x1b[36mFirst"));
}

#[test]
fn select_keeps_title_and_description_separated_with_empty_separator() {
    // :484 keeps title and description separated when the theme separator is empty
    let joined = join_pair("First", "", "one");
    assert!(joined.contains("First one"));
    assert!(!joined.contains("Firstone"));
}

#[test]
fn select_aligns_choices_when_paging_arrows_have_different_widths() {
    // :505 aligns choices when paging arrows have different widths
    let hidden = paging_prefix("", "vvv", false, false);
    let shown = paging_prefix("", "vvv", false, true);
    assert_eq!(hidden.chars().count(), shown.chars().count());
    assert_eq!(hidden, "   ");
    assert_eq!(shown, "vvv");
}

// --- Prompt.Password (:529-:572) ---

#[test]
fn password_renders_the_mask_from_the_prompt_theme() {
    // :529 renders the password mask from the prompt theme
    assert_eq!(mask_text("abc", "•"), "•••");
    assert_ne!(mask_text("abc", "•"), "***");
}

#[test]
fn password_keeps_cursor_movement_aligned_with_multi_char_mask() {
    // :543 keeps cursor movement aligned with a multi-character password mask
    assert_eq!(mask_text("abc", "**"), "******");
    assert_eq!(masked_cursor_offset(2, 3, 2), -2);
}

#[test]
fn password_starts_from_default_so_it_can_be_edited() {
    // :558 starts from the default value so it can be edited
    let mut buffer = TextBuffer::with_default("secret");
    buffer.type_text("123");
    assert_eq!(buffer.submit(), "secret123");
}

#[test]
fn password_does_not_submit_the_cleared_default_value() {
    // :572 does not submit the cleared default value
    let mut buffer = TextBuffer::with_default("secret");
    buffer.clear();
    assert_eq!(buffer.submit(), "");
}

// --- Prompt.Toggle (:588-:602) ---

#[test]
fn toggle_renders_the_separator_from_the_prompt_theme() {
    // :588 renders the separator from the prompt theme
    assert_eq!(toggle_text("on", "off", "|"), "on | off");
    assert_ne!(toggle_text("on", "off", "|"), "on / off");
}

#[test]
fn toggle_keeps_labels_separated_when_separator_is_empty() {
    // :602 keeps labels separated when the theme separator is empty
    assert_eq!(toggle_text("on", "off", ""), "on off");
    assert_ne!(toggle_text("on", "off", ""), "onoff");
}

// --- Prompt.AutoComplete (:618-:839) ---

fn fruits() -> Vec<&'static str> {
    vec!["Apple", "Banana", "Cherry"]
}

#[test]
fn autocomplete_filters_choices_as_you_type() {
    // :618 filters choices as you type
    assert_eq!(filter_matches(&fruits(), "ban"), vec![1]);
}

#[test]
fn autocomplete_backspace_widens_the_filter() {
    // :644 removes the last character on backspace
    let items = ["Alpha", "Beta", "Delta"];
    assert_eq!(filter_matches(&items, "al"), vec![0]);
    let mut query = TextBuffer::new();
    query.type_text("al");
    query.backspace();
    assert_eq!(filter_matches(&items, &query.submit()), vec![0, 1, 2]);
}

#[test]
fn autocomplete_clears_the_filter_input_on_ctrl_u() {
    // :673 clears the filter input on ctrl-u
    let items = ["Alpha", "Beta", "Delta"];
    assert_eq!(filter_matches(&items, "al"), vec![0]);
    assert_eq!(filter_matches(&items, ""), vec![0, 1, 2]);
}

#[test]
fn autocomplete_submit_with_no_matches_is_a_beep() {
    // :702 renders empty message and beeps on submit with no matches
    assert_eq!(submit_choice(&[false, false], &[], 0), None);
    assert_eq!(submit_choice(&[false, false], &[0, 1], 0), Some(0));
}

#[test]
fn autocomplete_beeps_when_submitting_a_disabled_choice() {
    // :729 beeps when submitting a disabled choice
    assert_eq!(submit_choice(&[true, false], &[0, 1], 0), None);
    assert_eq!(submit_choice(&[true, false], &[0, 1], 1), Some(1));
}

#[test]
fn autocomplete_empty_choice_list_never_submits() {
    // :750 renders empty message with no choices
    assert_eq!(submit_choice(&[], &[], 0), None);
}

#[test]
fn autocomplete_keeps_j_and_k_in_the_filter_query() {
    // :767 keeps `j` and `k` in the filter query instead of moving the cursor
    let items = ["feat/jira-fetch-tool", "main", "feat/worktree-tool"];
    assert_eq!(filter_matches(&items, "jira"), vec![0]);
}

#[test]
fn autocomplete_filters_a_query_that_merely_contains_k() {
    // :793 filters a query that merely contains a `k`
    let items = ["main", "feat/jira-fetch-tool", "feat/worktree-tool"];
    assert_eq!(filter_matches(&items, "worktree"), vec![2]);
}

#[test]
fn autocomplete_moves_the_cursor_with_ctrl_n_and_ctrl_p() {
    // :819 moves the cursor with ctrl-n and ctrl-p
    assert_eq!(step_index(0, 3, 1), 1);
    assert_eq!(step_index(1, 3, 1), 2);
    assert_eq!(step_index(2, 3, -1), 1);
}

#[test]
fn autocomplete_moves_the_cursor_up_with_ctrl_k() {
    // :839 moves the cursor up with ctrl-k
    assert_eq!(step_index(2, 3, -1), 1);
}

#[test]
fn select_cursor_wraps_around() {
    // :819/:839 cursor movement wraps at the ends, matching the pin's modulo
    assert_eq!(step_index(2, 3, 1), 0);
    assert_eq!(step_index(0, 3, -1), 2);
}

// --- Prompt.File (:881-:1017) ---

fn workspace_files() -> Vec<&'static str> {
    vec!["alpha.txt", "banana.txt", "basket.txt"]
}

#[test]
fn file_starts_from_the_default_value_so_it_can_be_submitted() {
    // :881 starts from the default value so it can be submitted
    let buffer = TextBuffer::with_default("/workspace/banana.txt");
    assert_eq!(buffer.submit(), "/workspace/banana.txt");
}

#[test]
fn file_filters_files_as_you_type() {
    // :894 filters files as you type
    let files = workspace_files();
    assert_eq!(filter_matches(&files, "ban"), vec![1]);
}

#[test]
fn file_backspace_widens_the_filter() {
    // :917 removes the last character on backspace
    let files = workspace_files();
    assert_eq!(filter_matches(&files, "ban"), vec![1]);
    assert_eq!(filter_matches(&files, "ba"), vec![1, 2]);
}

#[test]
fn file_keeps_j_in_the_filter_query() {
    // :957 keeps `j` in the filter query instead of moving the cursor
    let files = ["jest.config.ts", "package.json", "readme.md"];
    assert_eq!(filter_matches(&files, "jest"), vec![0]);
}

#[test]
fn file_keeps_k_in_the_filter_query() {
    // :979 keeps `k` in the filter query instead of moving the cursor
    let files = ["jest.config.ts", "package.json", "readme.md"];
    assert_eq!(filter_matches(&files, "package"), vec![1]);
}

#[test]
fn file_moves_the_cursor_with_ctrl_n_and_ctrl_p() {
    // :1001 moves the cursor with ctrl-n and ctrl-p
    assert_eq!(step_index(0, 3, 1), 1);
    assert_eq!(step_index(1, 3, 1), 2);
    assert_eq!(step_index(2, 3, -1), 1);
}

#[test]
fn file_moves_the_cursor_up_with_ctrl_k() {
    // :1017 moves the cursor up with ctrl-k
    assert_eq!(step_index(2, 3, -1), 1);
}

// --- Prompt.MultiSelect (:1035-:1192) ---

#[test]
fn multiselect_renders_only_the_choice_list_when_message_is_omitted() {
    // :1035 renders only the choice list when message is omitted
    assert_eq!(title_line_opt(None, "?", "›"), None);
}

#[test]
fn multiselect_min_validation_names_the_required_count() {
    // :1059 clears validation errors without a message; min pins "At least 1 are required"
    assert_eq!(
        multiselect_validate(0, Some(1), None),
        Some("At least 1 are required".to_owned())
    );
    assert_eq!(multiselect_validate(2, Some(1), None), None);
}

#[test]
fn multiselect_renders_paging_and_checkbox_symbols_from_the_theme() {
    // :1086 renders paging and checkbox symbols from the prompt theme
    let theme = PromptTheme {
        arrow_down: "v".to_owned(),
        checkbox_on: "[x]".to_owned(),
        checkbox_off: "[ ]".to_owned(),
        ..PromptTheme::default()
    };
    assert_eq!(
        paging_prefix(&theme.arrow_up, &theme.arrow_down, false, true),
        "v"
    );
    assert_eq!(checkbox_for(true, &theme), "[x]");
    assert_eq!(checkbox_for(false, &theme), "[ ]");
}

#[test]
fn multiselect_aligns_choices_when_checkbox_symbols_have_different_widths() {
    // :1114 aligns choices when checkbox symbols have different widths
    let width = 3;
    let alpha = format!("P{}{}", checkbox_cell("[x]", width), "Alpha");
    let beta = format!("P{}{}", checkbox_cell("", width), "Beta");
    assert_eq!(checkbox_cell("[x]", width), "[x] ");
    assert_eq!(checkbox_cell("", width), "    ");
    assert_eq!(alpha.find("Alpha"), beta.find("Beta"));
}

#[test]
fn multiselect_does_not_select_a_disabled_choice() {
    // :1137 does not allow a disabled multi-select choice to be selected
    assert_eq!(submit_choice(&[true], &[0], 0), None);
}

#[test]
fn multiselect_select_all_skips_disabled_choices() {
    // :1155 does not select disabled choices when selecting all
    assert_eq!(
        elm_effect_unstable::cli::prompt::select_all(2, &[false, true]),
        vec![0]
    );
    assert_eq!(select_all_label(true), "Select None");
    assert_eq!(select_all_label(false), "Select All");
}

#[test]
fn multiselect_invert_skips_disabled_choices() {
    // :1174 does not select disabled choices when inverting the selection
    assert_eq!(
        elm_effect_unstable::cli::prompt::invert_selection(&[1], 2, &[false, true]),
        vec![0]
    );
}

#[test]
fn multiselect_validation_ignores_disabled_preselected_choices() {
    // :1192 ignores disabled preselected choices when validating and submitting
    assert_eq!(multiselect_validate(1, None, Some(1)), None);
    assert_eq!(
        multiselect_validate(2, None, Some(1)),
        Some("At most 1 choices are allowed".to_owned())
    );
}
