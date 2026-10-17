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
            .with_subcommands(vec![
                DescriptorCommand::new("serve").with_description("Start the server"),
            ]),
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
pub type Golden = (
    &'static str,
    &'static str,
    fn() -> CommandDescriptor,
    &'static str,
);

/// Bash goldens, generated by the pin's `bash.ts`.
pub fn bash_goldens() -> Vec<Golden> {
    vec![
        (
            "tool",
            "tool",
            tool,
            include_str!("../fixtures/completions/bash/tool.bash"),
        ),
        (
            "simple",
            "greet",
            simple,
            include_str!("../fixtures/completions/bash/simple.bash"),
        ),
        (
            "server",
            "server",
            server,
            include_str!("../fixtures/completions/bash/server.bash"),
        ),
        (
            "choices",
            "deploy",
            choices,
            include_str!("../fixtures/completions/bash/choices.bash"),
        ),
        (
            "tricky",
            "deploy",
            tricky,
            include_str!("../fixtures/completions/bash/tricky.bash"),
        ),
        (
            "paths",
            "process",
            paths,
            include_str!("../fixtures/completions/bash/paths.bash"),
        ),
        (
            "optionaldir",
            "example",
            optionaldir,
            include_str!("../fixtures/completions/bash/optionaldir.bash"),
        ),
        (
            "nested",
            "top",
            nested,
            include_str!("../fixtures/completions/bash/nested.bash"),
        ),
        (
            "noop",
            "noop",
            noop,
            include_str!("../fixtures/completions/bash/noop.bash"),
        ),
        (
            "comprehensive",
            "mycli",
            comprehensive,
            include_str!("../fixtures/completions/bash/comprehensive.bash"),
        ),
    ]
}

/// Zsh goldens, generated by the pin's `zsh.ts`.
pub fn zsh_goldens() -> Vec<Golden> {
    vec![
        (
            "tool",
            "tool",
            tool,
            include_str!("../fixtures/completions/zsh/tool.zsh"),
        ),
        (
            "simple",
            "greet",
            simple,
            include_str!("../fixtures/completions/zsh/simple.zsh"),
        ),
        (
            "server",
            "server",
            server,
            include_str!("../fixtures/completions/zsh/server.zsh"),
        ),
        (
            "choices",
            "deploy",
            choices,
            include_str!("../fixtures/completions/zsh/choices.zsh"),
        ),
        (
            "tricky",
            "deploy",
            tricky,
            include_str!("../fixtures/completions/zsh/tricky.zsh"),
        ),
        (
            "paths",
            "process",
            paths,
            include_str!("../fixtures/completions/zsh/paths.zsh"),
        ),
        (
            "optionaldir",
            "example",
            optionaldir,
            include_str!("../fixtures/completions/zsh/optionaldir.zsh"),
        ),
        (
            "nested",
            "top",
            nested,
            include_str!("../fixtures/completions/zsh/nested.zsh"),
        ),
        (
            "noop",
            "noop",
            noop,
            include_str!("../fixtures/completions/zsh/noop.zsh"),
        ),
        (
            "comprehensive",
            "mycli",
            comprehensive,
            include_str!("../fixtures/completions/zsh/comprehensive.zsh"),
        ),
    ]
}

/// `tool` with two subcommands that each own a `common` leaf (row :552).
pub fn scoped() -> CommandDescriptor {
    let leaf = |name: &str, flag_name: &str| CommandDescriptor {
        name: name.to_owned(),
        description: None,
        flags: vec![flag(flag_name, &[], FlagType::Boolean)],
        arguments: vec![],
        subcommands: vec![],
    };
    CommandDescriptor {
        name: "tool".to_owned(),
        description: None,
        flags: vec![],
        arguments: vec![],
        subcommands: vec![
            CommandDescriptor {
                name: "alpha".to_owned(),
                description: None,
                flags: vec![],
                arguments: vec![],
                subcommands: vec![leaf("common", "alpha-only")],
            },
            CommandDescriptor {
                name: "beta".to_owned(),
                description: None,
                flags: vec![],
                arguments: vec![],
                subcommands: vec![leaf("common", "beta-only")],
            },
        ],
    }
}

/// `deploy` whose flag description ends in a backslash (row :626).
pub fn backslash_desc() -> CommandDescriptor {
    CommandDescriptor {
        name: "deploy".to_owned(),
        description: None,
        flags: vec![FlagDescriptor {
            name: "mode".to_owned(),
            aliases: vec![],
            description: Some("Path like C:\\".to_owned()),
            flag_type: FlagType::Choice(vec!["a".to_owned()]),
        }],
        arguments: vec![],
        subcommands: vec![],
    }
}

/// The pin's `test/cli/fixtures/ComprehensiveCli.ts`, transcribed whole.
///
/// Commands built with `Command.withSharedFlags` carry those flags twice, as
/// the pin does: once in `flags` (parsed here) and once in `context_flags`
/// (inherited by children).
pub fn comprehensive() -> CommandDescriptor {
    let sudo =
        ParamDef::flag("sudo", ParamType::Boolean).with_description("Run with elevated privileges");

    let users_list = DescriptorCommand::new("list")
        .with_description("List all users in the system")
        .with_flags(vec![
            ParamDef::flag("format", ParamType::String)
                .with_description("Output format (json, table, csv)"),
            ParamDef::flag("active", ParamType::Boolean).with_description("Show only active users"),
            ParamDef::flag("verbose", ParamType::Boolean)
                .with_alias("v")
                .with_description("Show detailed information"),
        ]);
    let users_create = DescriptorCommand::new("create")
        .with_description("Create a new user account")
        .with_flags(vec![
            ParamDef::flag("role", ParamType::String)
                .with_description("User role (admin, user, guest)"),
            ParamDef::flag("notify", ParamType::Boolean)
                .with_alias("n")
                .with_description("Send notification email"),
        ])
        .with_arguments(vec![
            ParamDef::argument("username", ParamType::String)
                .with_description("Username for the new user"),
            ParamDef::argument("email", ParamType::String)
                .with_description("Email address (optional)")
                .optional(),
        ]);
    let users = DescriptorCommand::new("users")
        .with_description("User management commands")
        .with_subcommands(vec![users_list, users_create]);

    let profile = ParamDef::flag("profile", ParamType::String)
        .with_alias("p")
        .with_description("Configuration profile to use");
    let config_set = DescriptorCommand::new("set")
        .with_description("Set configuration values")
        .with_flags(vec![
            ParamDef::flag("config-file", ParamType::Path(PathKind::File))
                .with_alias("f")
                .with_description("Write to specific config file"),
        ])
        .with_arguments(vec![
            ParamDef::argument("key=value", ParamType::String)
                .with_description("Configuration key-value pairs")
                .variadic(),
        ]);
    let config_get = DescriptorCommand::new("get")
        .with_description("Get configuration value")
        .with_flags(vec![
            ParamDef::flag("source", ParamType::String)
                .with_description("Configuration source (local, global, system)"),
        ])
        .with_arguments(vec![
            ParamDef::argument("key", ParamType::String)
                .with_description("Configuration key to retrieve"),
        ]);
    let config = DescriptorCommand::new("config")
        .with_description("Manage application configuration")
        .with_flags(vec![profile.clone()])
        .with_context_flags(vec![profile])
        .with_subcommands(vec![config_set, config_get]);

    let admin = DescriptorCommand::new("admin")
        .with_description("Administrative commands")
        .with_flags(vec![sudo.clone()])
        .with_context_flags(vec![sudo])
        .with_subcommands(vec![users, config]);

    let copy = DescriptorCommand::new("copy")
        .with_description("Copy files or directories")
        .with_flags(vec![
            ParamDef::flag("recursive", ParamType::Boolean)
                .with_alias("r")
                .with_description("Copy directories recursively"),
            ParamDef::flag("force", ParamType::Boolean)
                .with_alias("f")
                .with_description("Overwrite existing files"),
            ParamDef::flag("buffer-size", ParamType::Int).with_description("Buffer size in KB"),
        ])
        .with_arguments(vec![
            ParamDef::argument("source", ParamType::Path(PathKind::File))
                .with_description("Source file or directory"),
            ParamDef::argument("destination", ParamType::Path(PathKind::File))
                .with_description("Destination path"),
        ]);

    let move_ = DescriptorCommand::new("move")
        .with_description("Move or rename files")
        .with_flags(vec![
            ParamDef::flag("interactive", ParamType::Boolean)
                .with_alias("i")
                .with_description("Prompt before overwrite"),
        ])
        .with_arguments(vec![
            ParamDef::argument("paths", ParamType::String)
                .with_description("Source path(s) and destination")
                .variadic(),
        ]);

    let remove = DescriptorCommand::new("remove")
        .with_description("Remove files or directories")
        .with_flags(vec![
            ParamDef::flag("recursive", ParamType::Boolean)
                .with_alias("r")
                .with_description("Remove directories and contents"),
            ParamDef::flag("force", ParamType::Boolean)
                .with_alias("f")
                .with_description("Force removal without prompts"),
            ParamDef::flag("verbose", ParamType::Boolean)
                .with_alias("v")
                .with_description("Explain what is being done"),
        ])
        .with_arguments(vec![
            ParamDef::argument("files", ParamType::String)
                .with_description("Files to remove")
                .variadic(),
        ]);

    let build = DescriptorCommand::new("build")
        .with_description("Build the project")
        .with_flags(vec![
            ParamDef::flag("output", ParamType::String)
                .with_alias("o")
                .with_description("Output directory"),
            ParamDef::flag("verbose", ParamType::Boolean)
                .with_alias("v")
                .with_description("Enable verbose output"),
            ParamDef::flag("config-file", ParamType::String)
                .with_alias("f")
                .with_description("Configuration file path"),
        ]);

    let git_clone = DescriptorCommand::new("clone")
        .with_description("Clone a repository")
        .with_flags(vec![
            ParamDef::flag("branch", ParamType::String).with_description("Branch to clone"),
        ])
        .with_arguments(vec![
            ParamDef::argument("repository", ParamType::String)
                .with_description("Repository URL or path"),
        ]);
    let git_add = DescriptorCommand::new("add")
        .with_description("Add files to staging")
        .with_flags(vec![
            ParamDef::flag("update", ParamType::Boolean).with_description("Update tracked files"),
        ])
        .with_arguments(vec![
            ParamDef::argument("files", ParamType::String).with_description("Files to add"),
        ]);
    let git_status = DescriptorCommand::new("status")
        .with_description("Show repository status")
        .with_flags(vec![
            ParamDef::flag("short", ParamType::Boolean).with_description("Show short format"),
        ]);
    let git_verbose =
        ParamDef::flag("verbose", ParamType::Boolean).with_description("Enable verbose output");
    let git = DescriptorCommand::new("git")
        .with_description("Git version control")
        .with_flags(vec![git_verbose.clone()])
        .with_context_flags(vec![git_verbose])
        .with_subcommands(vec![git_clone, git_add, git_status]);

    let test_required = DescriptorCommand::new("test-required")
        .with_description("Test command with required option")
        .with_flags(vec![
            ParamDef::flag("required", ParamType::String)
                .with_description("A required option for testing"),
        ]);
    let test_failing = DescriptorCommand::new("test-failing")
        .with_description("Test command that always fails")
        .with_flags(vec![
            ParamDef::flag("input", ParamType::String)
                .with_description("Input that will cause handler to fail"),
        ]);

    let deploy = DescriptorCommand::new("deploy")
        .with_description("Deploy a service")
        .with_flags(vec![
            ParamDef::flag("db-host", ParamType::String).with_description("Database host"),
            ParamDef::flag("db-port", ParamType::Int).with_description("Database port"),
            ParamDef::flag("dry-run", ParamType::Boolean).with_description("Perform a dry run"),
        ])
        .with_arguments(vec![
            ParamDef::argument("service", ParamType::String).with_description("Service to deploy"),
            ParamDef::argument("environment", ParamType::String)
                .with_description("Target environment"),
        ]);

    let app_env = ParamDef::flag("env", ParamType::String).with_description("Environment setting");
    let app = DescriptorCommand::new("app")
        .with_description("Application management")
        .with_flags(vec![app_env.clone()])
        .with_context_flags(vec![app_env])
        .with_subcommands(vec![deploy.clone()]);

    let service_name = ParamDef::flag("name", ParamType::String).with_description("Service name");
    let service = DescriptorCommand::new("service")
        .with_description("Service management")
        .with_flags(vec![service_name.clone()])
        .with_context_flags(vec![service_name])
        .with_subcommands(vec![deploy]);
    let nested_env =
        ParamDef::flag("env", ParamType::String).with_description("Environment setting");
    let app_with_service = DescriptorCommand::new("app-nested")
        .with_description("Application with nested services")
        .with_flags(vec![nested_env.clone()])
        .with_context_flags(vec![nested_env])
        .with_subcommands(vec![service]);

    let global = vec![
        ParamDef::flag("debug", ParamType::Boolean)
            .with_alias("d")
            .with_description("Enable debug logging"),
        ParamDef::flag("config", ParamType::Path(PathKind::File))
            .with_alias("c")
            .with_description("Path to configuration file"),
        ParamDef::flag("quiet", ParamType::Boolean)
            .with_alias("q")
            .with_description("Suppress non-error output"),
    ];
    from_command(
        &DescriptorCommand::new("mycli")
            .with_description("A comprehensive CLI tool demonstrating all features")
            .with_flags(global.clone())
            .with_context_flags(global)
            .with_subcommands(vec![
                admin,
                copy,
                move_,
                remove,
                build,
                git,
                test_required,
                test_failing,
                app,
                app_with_service,
            ]),
        &[],
    )
}

/// Fish goldens, generated by the pin's `fish.ts`.
pub fn fish_goldens() -> Vec<Golden> {
    vec![
        (
            "tool",
            "tool",
            tool,
            include_str!("../fixtures/completions/fish/tool.fish"),
        ),
        (
            "simple",
            "greet",
            simple,
            include_str!("../fixtures/completions/fish/simple.fish"),
        ),
        (
            "server",
            "server",
            server,
            include_str!("../fixtures/completions/fish/server.fish"),
        ),
        (
            "choices",
            "deploy",
            choices,
            include_str!("../fixtures/completions/fish/choices.fish"),
        ),
        (
            "tricky",
            "deploy",
            tricky,
            include_str!("../fixtures/completions/fish/tricky.fish"),
        ),
        (
            "paths",
            "process",
            paths,
            include_str!("../fixtures/completions/fish/paths.fish"),
        ),
        (
            "optionaldir",
            "example",
            optionaldir,
            include_str!("../fixtures/completions/fish/optionaldir.fish"),
        ),
        (
            "nested",
            "top",
            nested,
            include_str!("../fixtures/completions/fish/nested.fish"),
        ),
        (
            "noop",
            "noop",
            noop,
            include_str!("../fixtures/completions/fish/noop.fish"),
        ),
        (
            "comprehensive",
            "mycli",
            comprehensive,
            include_str!("../fixtures/completions/fish/comprehensive.fish"),
        ),
        (
            "scoped",
            "tool",
            scoped,
            include_str!("../fixtures/completions/fish/scoped.fish"),
        ),
        (
            "backslash_desc",
            "deploy",
            backslash_desc,
            include_str!("../fixtures/completions/fish/backslash_desc.fish"),
        ),
    ]
}
