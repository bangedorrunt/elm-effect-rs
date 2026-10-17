//! Shared command-descriptor fixtures and pin-generated goldens for the
//! completion-generator witnesses (bash, zsh, fish).
//!
//! Each descriptor mirrors the command builder of the same name in
//! `packages/effect/test/cli/completions/completions.test.ts`, and each golden
//! was produced by the pin's own `internal/completions/*.ts` generator.

use elm_effect_unstable::cli::primitive::PathKind;
use elm_effect_unstable::cli_completions::{
    ArgumentDescriptor, ArgumentType, CommandDescriptor, DescriptorCommand, FlagDescriptor,
    FlagType, ParamDef, ParamType, from_command,
};

pub fn flag(name: &str, aliases: &[&str], flag_type: FlagType) -> FlagDescriptor {
    FlagDescriptor {
        name: name.to_owned(),
        aliases: aliases.iter().map(|a| (*a).to_owned()).collect(),
        description: None,
        flag_type,
    }
}

pub fn argument(name: &str, argument_type: ArgumentType) -> ArgumentDescriptor {
    ArgumentDescriptor {
        name: name.to_owned(),
        description: None,
        required: true,
        variadic: false,
        argument_type,
    }
}

fn tricky_values() -> Vec<String> {
    [
        "it's-fine",
        "node:20",
        "with space",
        "(whoami)",
        "#tag",
        "$HOME",
        "back\\slash",
        "say\"hi\"",
        "a*b",
        "a;b",
        "~x",
        "foo'",
        "a!b",
        "\u{1F680}",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

pub fn tool() -> CommandDescriptor {
    CommandDescriptor {
        name: "tool".to_owned(),
        description: None,
        flags: vec![
            flag("verbose", &["v"], FlagType::Boolean),
            flag(
                "format",
                &["f"],
                FlagType::Choice(vec!["json".to_owned(), "text".to_owned()]),
            ),
        ],
        arguments: vec![
            argument("source", ArgumentType::Choice(vec!["one".to_owned()])),
            argument("target", ArgumentType::Choice(vec!["two".to_owned()])),
        ],
        subcommands: vec![],
    }
}

pub fn simple() -> CommandDescriptor {
    from_command(
        &DescriptorCommand::new("greet")
            .with_description("Greet someone")
            .with_flags(vec![
                ParamDef::flag("loud", ParamType::Boolean)
                    .with_alias("l")
                    .with_description("Shout the greeting"),
                ParamDef::flag("times", ParamType::Int).with_description("Repeat count"),
            ])
            .with_arguments(vec![
                ParamDef::argument("name", ParamType::String).with_description("Name to greet"),
            ]),
        &[],
    )
}

pub fn server() -> CommandDescriptor {
    let start = DescriptorCommand::new("start")
        .with_description("Start the server")
        .with_flags(vec![
            ParamDef::flag("port", ParamType::Int)
                .with_alias("p")
                .with_description("Port number"),
            ParamDef::flag("daemon", ParamType::Boolean).with_description("Run as daemon"),
        ]);
    let stop = DescriptorCommand::new("stop")
        .with_description("Stop the server")
        .with_flags(vec![
            ParamDef::flag("force", ParamType::Boolean)
                .with_alias("f")
                .with_description("Force stop"),
        ]);
    from_command(
        &DescriptorCommand::new("server")
            .with_description("Server management")
            .with_flags(vec![
                ParamDef::flag("verbose", ParamType::Boolean).with_alias("v"),
                ParamDef::flag("config", ParamType::String),
            ])
            .with_subcommands(vec![start, stop]),
        &[],
    )
}

pub fn choices() -> CommandDescriptor {
    from_command(
        &DescriptorCommand::new("deploy")
            .with_description("Deploy application")
            .with_flags(vec![
                ParamDef::flag(
                    "env",
                    ParamType::Choice(vec![
                        "dev".to_owned(),
                        "staging".to_owned(),
                        "prod".to_owned(),
                    ]),
                )
                .with_description("Target environment"),
            ])
            .with_arguments(vec![
                ParamDef::argument(
                    "region",
                    ParamType::Choice(vec![
                        "us-east".to_owned(),
                        "eu-west".to_owned(),
                        "ap-south".to_owned(),
                    ]),
                )
                .with_description("Deployment region"),
            ]),
        &[],
    )
}

pub fn tricky() -> CommandDescriptor {
    from_command(
        &DescriptorCommand::new("deploy")
            .with_description("Deploy application")
            .with_flags(vec![
                ParamDef::flag("mode", ParamType::Choice(tricky_values()))
                    .with_description("Deploy mode"),
            ])
            .with_arguments(vec![
                ParamDef::argument(
                    "target",
                    ParamType::Choice(
                        ["o'clock", "a:b", "{x,y}", "a\u{1F600}b"]
                            .iter()
                            .map(|s| (*s).to_owned())
                            .collect(),
                    ),
                )
                .with_description("Deployment target"),
            ]),
        &[],
    )
}

pub fn paths() -> CommandDescriptor {
    from_command(
        &DescriptorCommand::new("process")
            .with_description("Process files")
            .with_flags(vec![
                ParamDef::flag("input", ParamType::Path(PathKind::File))
                    .with_description("Input file"),
                ParamDef::flag("output-dir", ParamType::Path(PathKind::Directory))
                    .with_description("Output directory"),
            ])
            .with_arguments(vec![
                ParamDef::argument("source", ParamType::Path(PathKind::File))
                    .with_description("Source file"),
            ]),
        &[],
    )
}

pub fn optionaldir() -> CommandDescriptor {
    from_command(
        &DescriptorCommand::new("example")
            .with_arguments(vec![
                ParamDef::argument("directory", ParamType::Path(PathKind::Directory))
                    .with_description("Directory to start in")
                    .optional(),
            ])
            .with_subcommands(vec![DescriptorCommand::new("serve").with_description("Start the server")]),
        &[],
    )
}

pub fn nested() -> CommandDescriptor {
    let leaf = DescriptorCommand::new("action")
        .with_description("Perform action")
        .with_flags(vec![
            ParamDef::flag("dry-run", ParamType::Boolean).with_description("Dry run mode"),
        ]);
    let mid = DescriptorCommand::new("sub").with_subcommands(vec![leaf]);
    from_command(
        &DescriptorCommand::new("top").with_subcommands(vec![mid]),
        &[],
    )
}

pub fn noop() -> CommandDescriptor {
    from_command(
        &DescriptorCommand::new("noop").with_description("Does nothing"),
        &[],
    )
}

/// A fixture row: label, executable name, descriptor, and the pin's golden.
pub type Golden = (&'static str, &'static str, fn() -> CommandDescriptor, &'static str);

/// Bash goldens, generated by the pin's `bash.ts`.
pub fn bash_goldens() -> Vec<Golden> {
    vec![
        ("tool", "tool", tool, include_str!("../fixtures/completions/bash/tool.bash")),
        ("simple", "greet", simple, include_str!("../fixtures/completions/bash/simple.bash")),
        ("server", "server", server, include_str!("../fixtures/completions/bash/server.bash")),
        ("choices", "deploy", choices, include_str!("../fixtures/completions/bash/choices.bash")),
        ("tricky", "deploy", tricky, include_str!("../fixtures/completions/bash/tricky.bash")),
        ("paths", "process", paths, include_str!("../fixtures/completions/bash/paths.bash")),
        (
            "optionaldir",
            "example",
            optionaldir,
            include_str!("../fixtures/completions/bash/optionaldir.bash"),
        ),
        ("nested", "top", nested, include_str!("../fixtures/completions/bash/nested.bash")),
        ("noop", "noop", noop, include_str!("../fixtures/completions/bash/noop.bash")),
    ]
}

/// Zsh goldens, generated by the pin's `zsh.ts`.
pub fn zsh_goldens() -> Vec<Golden> {
    vec![
        ("tool", "tool", tool, include_str!("../fixtures/completions/zsh/tool.zsh")),
        ("simple", "greet", simple, include_str!("../fixtures/completions/zsh/simple.zsh")),
        ("server", "server", server, include_str!("../fixtures/completions/zsh/server.zsh")),
        ("choices", "deploy", choices, include_str!("../fixtures/completions/zsh/choices.zsh")),
        ("tricky", "deploy", tricky, include_str!("../fixtures/completions/zsh/tricky.zsh")),
        ("paths", "process", paths, include_str!("../fixtures/completions/zsh/paths.zsh")),
        (
            "optionaldir",
            "example",
            optionaldir,
            include_str!("../fixtures/completions/zsh/optionaldir.zsh"),
        ),
        ("nested", "top", nested, include_str!("../fixtures/completions/zsh/nested.zsh")),
        ("noop", "noop", noop, include_str!("../fixtures/completions/zsh/noop.zsh")),
    ]
}
