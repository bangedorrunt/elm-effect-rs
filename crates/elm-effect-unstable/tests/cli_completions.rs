//! `cli/completions/descriptor` witnesses for
//! `packages/effect/test/cli/completions/descriptor.test.ts`.
//!
//! Each test names the upstream line it proves. The descriptor is pure data:
//! a command tree in, plain flag/argument/subcommand rows out.

use elm_effect_unstable::cli::primitive::PathKind;
use elm_effect_unstable::cli_completions::{
    ArgumentType, DescriptorCommand, FlagType, ParamDef, ParamType, from_command,
};

#[test]
fn extracts_command_name_and_description() {
    // :8
    let cmd = DescriptorCommand::new("deploy").with_description("Deploy the application");
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.name, "deploy");
    assert_eq!(desc.description.as_deref(), Some("Deploy the application"));
}

#[test]
fn extracts_string_flags_with_aliases() {
    // :17
    let cmd = DescriptorCommand::new("test").with_flags(vec![
        ParamDef::flag("output", ParamType::String)
            .with_alias("o")
            .with_description("Output directory"),
    ]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.flags.len(), 1);
    assert_eq!(desc.flags[0].name, "output");
    assert_eq!(desc.flags[0].aliases, vec!["o".to_string()]);
    assert_eq!(desc.flags[0].description.as_deref(), Some("Output directory"));
    assert_eq!(desc.flags[0].flag_type, FlagType::String);
}

#[test]
fn extracts_boolean_flags() {
    // :32
    let cmd = DescriptorCommand::new("test")
        .with_flags(vec![ParamDef::flag("verbose", ParamType::Boolean).with_alias("v")]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.flags.len(), 1);
    assert_eq!(desc.flags[0].name, "verbose");
    assert_eq!(desc.flags[0].flag_type, FlagType::Boolean);
    assert_eq!(desc.flags[0].aliases, vec!["v".to_string()]);
}

#[test]
fn extracts_integer_and_float_flags() {
    // :43
    let cmd = DescriptorCommand::new("test").with_flags(vec![
        ParamDef::flag("port", ParamType::Int),
        ParamDef::flag("ratio", ParamType::Finite),
    ]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.flags.len(), 2);
    let port = desc.flags.iter().find(|f| f.name == "port").unwrap();
    let ratio = desc.flags.iter().find(|f| f.name == "ratio").unwrap();
    assert_eq!(port.flag_type, FlagType::Int);
    assert_eq!(ratio.flag_type, FlagType::Finite);
}

#[test]
fn extracts_file_directory_and_path_flags() {
    // :56
    let cmd = DescriptorCommand::new("test").with_flags(vec![
        ParamDef::flag("input", ParamType::Path(PathKind::File)),
        ParamDef::flag("out-dir", ParamType::Path(PathKind::Directory)),
        ParamDef::flag("config", ParamType::Path(PathKind::Either)),
    ]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.flags.len(), 3);
    assert_eq!(
        desc.flags.iter().find(|f| f.name == "input").unwrap().flag_type,
        FlagType::Path(PathKind::File)
    );
    assert_eq!(
        desc.flags.iter().find(|f| f.name == "out-dir").unwrap().flag_type,
        FlagType::Path(PathKind::Directory)
    );
    assert_eq!(
        desc.flags.iter().find(|f| f.name == "config").unwrap().flag_type,
        FlagType::Path(PathKind::Either)
    );
}

#[test]
fn retains_path_semantics_when_a_path_has_a_custom_metavar() {
    // :72 — withMetavar changes the label, never the value shape.
    let cmd = DescriptorCommand::new("app")
        .with_flags(vec![
            ParamDef::flag("directory", ParamType::Path(PathKind::Directory))
                .with_metavar("DIR"),
        ])
        .with_arguments(vec![
            ParamDef::argument("file", ParamType::Path(PathKind::File)).with_metavar("INPUT"),
        ]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(
        desc.flags[0].flag_type,
        FlagType::Path(PathKind::Directory)
    );
    assert_eq!(
        desc.arguments[0].argument_type,
        ArgumentType::Path(PathKind::File)
    );
}

#[test]
fn classifies_file_backed_flags_and_arguments_as_file_paths() {
    // :83
    let cmd = DescriptorCommand::new("app")
        .with_flags(vec![
            ParamDef::flag("flag-text", ParamType::FileText),
            ParamDef::flag("flag-parse", ParamType::FileParse),
            ParamDef::flag("flag-schema", ParamType::FileSchema),
        ])
        .with_arguments(vec![
            ParamDef::argument("argument-text", ParamType::FileText),
            ParamDef::argument("argument-parse", ParamType::FileParse),
            ParamDef::argument("argument-schema", ParamType::FileSchema),
        ]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(
        desc.flags.iter().map(|f| f.flag_type.clone()).collect::<Vec<_>>(),
        vec![
            FlagType::Path(PathKind::File),
            FlagType::Path(PathKind::File),
            FlagType::Path(PathKind::File)
        ]
    );
    assert_eq!(
        desc.arguments
            .iter()
            .map(|a| a.argument_type.clone())
            .collect::<Vec<_>>(),
        vec![
            ArgumentType::Path(PathKind::File),
            ArgumentType::Path(PathKind::File),
            ArgumentType::Path(PathKind::File)
        ]
    );
}

#[test]
fn extracts_choice_flags_with_values() {
    // :99
    let cmd = DescriptorCommand::new("test").with_flags(vec![ParamDef::flag(
        "color",
        ParamType::Choice(vec!["red".into(), "green".into(), "blue".into()]),
    )]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.flags.len(), 1);
    assert_eq!(
        desc.flags[0].flag_type,
        FlagType::Choice(vec!["red".into(), "green".into(), "blue".into()])
    );
}

#[test]
fn extracts_optional_flags() {
    // :111
    let cmd = DescriptorCommand::new("test")
        .with_flags(vec![ParamDef::flag("name", ParamType::String).optional()]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.flags.len(), 1);
    assert_eq!(desc.flags[0].name, "name");
    assert_eq!(desc.flags[0].flag_type, FlagType::String);
}

#[test]
fn extracts_positional_arguments_with_types() {
    // :121
    let cmd = DescriptorCommand::new("test").with_arguments(vec![
        ParamDef::argument("name", ParamType::String),
        ParamDef::argument("count", ParamType::Int),
    ]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.arguments.len(), 2);
    let name = desc.arguments.iter().find(|a| a.name == "name").unwrap();
    let count = desc.arguments.iter().find(|a| a.name == "count").unwrap();
    assert_eq!(name.argument_type, ArgumentType::String);
    assert_eq!(count.argument_type, ArgumentType::Int);
    assert!(name.required);
    assert!(count.required);
}

#[test]
fn extracts_variadic_arguments() {
    // :136
    let cmd = DescriptorCommand::new("test")
        .with_arguments(vec![ParamDef::argument("files", ParamType::String).variadic()]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.arguments.len(), 1);
    assert_eq!(desc.arguments[0].name, "files");
    assert!(desc.arguments[0].variadic);
    assert!(desc.arguments[0].required);
}

#[test]
fn extracts_optional_arguments() {
    // :147
    let cmd = DescriptorCommand::new("test")
        .with_arguments(vec![ParamDef::argument("email", ParamType::String).optional()]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.arguments.len(), 1);
    assert_eq!(desc.arguments[0].name, "email");
    assert!(!desc.arguments[0].required);
}

#[test]
fn extracts_nested_subcommands_recursively() {
    // :157
    let leaf = DescriptorCommand::new("leaf").with_description("A leaf command");
    let mid = DescriptorCommand::new("mid").with_subcommands(vec![leaf]);
    let root = DescriptorCommand::new("root").with_subcommands(vec![mid]);
    let desc = from_command(&root, &[]);
    assert_eq!(desc.subcommands.len(), 1);
    assert_eq!(desc.subcommands[0].name, "mid");
    assert_eq!(desc.subcommands[0].subcommands.len(), 1);
    assert_eq!(desc.subcommands[0].subcommands[0].name, "leaf");
    assert_eq!(
        desc.subcommands[0].subcommands[0].description.as_deref(),
        Some("A leaf command")
    );
}

#[test]
fn extracts_descriptions_from_flags_and_arguments() {
    // :175
    let cmd = DescriptorCommand::new("test")
        .with_flags(vec![ParamDef::flag("port", ParamType::Int).with_description("Port number")])
        .with_arguments(vec![
            ParamDef::argument("host", ParamType::String).with_description("Hostname"),
        ]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.flags[0].description.as_deref(), Some("Port number"));
    assert_eq!(desc.arguments[0].description.as_deref(), Some("Hostname"));
}

#[test]
fn handles_commands_with_no_flags_or_arguments() {
    // :189
    let desc = from_command(&DescriptorCommand::new("noop"), &[]);
    assert_eq!(desc.name, "noop");
    assert!(desc.flags.is_empty());
    assert!(desc.arguments.is_empty());
    assert!(desc.subcommands.is_empty());
}

#[test]
fn handles_deeply_nested_command_trees() {
    // :198
    let level3 = DescriptorCommand::new("level3")
        .with_flags(vec![ParamDef::flag("deep", ParamType::Boolean)]);
    let level2 = DescriptorCommand::new("level2").with_subcommands(vec![level3]);
    let level1 = DescriptorCommand::new("level1").with_subcommands(vec![level2]);
    let root = DescriptorCommand::new("root").with_subcommands(vec![level1]);
    let desc = from_command(&root, &[]);
    let deep = &desc.subcommands[0].subcommands[0].subcommands[0];
    assert_eq!(deep.name, "level3");
    assert_eq!(deep.flags.len(), 1);
    assert_eq!(deep.flags[0].name, "deep");
    assert_eq!(deep.flags[0].flag_type, FlagType::Boolean);
}

#[test]
fn extracts_choice_arguments() {
    // :219
    let cmd = DescriptorCommand::new("test").with_arguments(vec![ParamDef::argument(
        "env",
        ParamType::Choice(vec!["dev".into(), "staging".into(), "prod".into()]),
    )]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.arguments.len(), 1);
    assert_eq!(
        desc.arguments[0].argument_type,
        ArgumentType::Choice(vec!["dev".into(), "staging".into(), "prod".into()])
    );
}

#[test]
fn extracts_file_directory_arguments() {
    // :231
    let cmd = DescriptorCommand::new("test").with_arguments(vec![
        ParamDef::argument("input", ParamType::Path(PathKind::File)),
        ParamDef::argument("output", ParamType::Path(PathKind::Directory)),
    ]);
    let desc = from_command(&cmd, &[]);
    assert_eq!(desc.arguments.len(), 2);
    assert_eq!(
        desc.arguments.iter().find(|a| a.name == "input").unwrap().argument_type,
        ArgumentType::Path(PathKind::File)
    );
    assert_eq!(
        desc.arguments.iter().find(|a| a.name == "output").unwrap().argument_type,
        ArgumentType::Path(PathKind::Directory)
    );
}

#[test]
fn extracts_multiple_subcommands() {
    // :244
    let root = DescriptorCommand::new("root").with_subcommands(vec![
        DescriptorCommand::new("alpha").with_description("First"),
        DescriptorCommand::new("beta").with_description("Second"),
        DescriptorCommand::new("gamma").with_description("Third"),
    ]);
    let desc = from_command(&root, &[]);
    assert_eq!(desc.subcommands.len(), 3);
    assert_eq!(desc.subcommands[0].name, "alpha");
    assert_eq!(desc.subcommands[1].name, "beta");
    assert_eq!(desc.subcommands[2].name, "gamma");
}

#[test]
fn prefers_short_descriptions_for_subcommand_listings() {
    // :258
    let root = DescriptorCommand::new("root").with_subcommands(vec![
        DescriptorCommand::new("build")
            .with_description("Build the project and all artifacts")
            .with_short_description("Build artifacts"),
        DescriptorCommand::new("test").with_description("Run the full test suite"),
    ]);
    let desc = from_command(&root, &[]);
    assert_eq!(desc.subcommands[0].description.as_deref(), Some("Build artifacts"));
    assert_eq!(
        desc.subcommands[1].description.as_deref(),
        Some("Run the full test suite")
    );
}
