//! `cli/Help` witnesses for `packages/effect/test/cli/Help.test.ts`.
//!
//! Each test names the upstream line it proves. The port covers the pure
//! core of the TS rows: the `HelpDoc` model (flags with required markers,
//! hidden/unlisted filtering, aliases, groups, examples) plus the plain-text
//! formatter with display-width column alignment (CJK width 2, zero-width
//! marks 0). The TS effect plumbing (layers, `TestConsole`, `runWith`)
//! has no Rust counterpart; the model plus its rendering is the behavior
//! the rows pin.

use elm_effect_unstable::cli::help::{
    HelpArgDecl, HelpExample, HelpFlagDecl, HelpGroup, HelpNode, format_help, help_for_path,
    visual_width,
};
use elm_effect_unstable::cli::suggest;
use elm_effect_unstable::cli_completions::{
    DescriptorCommand, ParamDef, ParamType, bash, from_command,
};

/// The pin's global shared flags on the comprehensive root.
fn root_shared_flags() -> Vec<HelpFlagDecl> {
    vec![
        HelpFlagDecl::boolean("debug")
            .with_alias("d")
            .with_description("Enable debug logging")
            .with_default(),
        HelpFlagDecl::file("config")
            .with_alias("c")
            .with_description("Path to configuration file")
            .optional(),
        HelpFlagDecl::boolean("quiet")
            .with_alias("q")
            .with_description("Suppress non-error output")
            .with_default(),
    ]
}

/// The comprehensive fixture CLI (`fixtures/ComprehensiveCli.ts`), trimmed to
/// the surface the help rows observe: names, descriptions, flags, args,
/// nesting, and shared flags.
fn comprehensive_mycli() -> HelpNode {
    let users_list = HelpNode::new("list")
        .with_description("List all users in the system")
        .with_flags(vec![
            HelpFlagDecl::string("format")
                .with_description("Output format (json, table, csv)")
                .with_default(),
            HelpFlagDecl::boolean("active")
                .with_description("Show only active users")
                .with_default(),
            HelpFlagDecl::boolean("verbose")
                .with_alias("v")
                .with_description("Show detailed information")
                .with_default(),
        ]);
    let users_create = HelpNode::new("create")
        .with_description("Create a new user account")
        .with_args(vec![
            HelpArgDecl::string("username").with_description("Username for the new user"),
            HelpArgDecl::string("email")
                .with_description("Email address (optional)")
                .optional(),
        ])
        .with_flags(vec![
            HelpFlagDecl::string("role").with_description("User role (admin, user, guest)"),
            HelpFlagDecl::boolean("notify")
                .with_alias("n")
                .with_description("Send notification email")
                .with_default(),
        ]);
    let users = HelpNode::new("users")
        .with_description("User management commands")
        .with_subcommands(vec![users_list, users_create]);
    let config_set = HelpNode::new("set")
        .with_description("Set configuration values")
        .with_args(vec![
            HelpArgDecl::string("key=value")
                .with_description("Configuration key-value pairs")
                .variadic_min(1),
        ])
        .with_flags(vec![
            HelpFlagDecl::file("config-file")
                .with_alias("f")
                .with_description("Write to specific config file")
                .optional(),
        ]);
    let config_get = HelpNode::new("get")
        .with_description("Get configuration value")
        .with_args(vec![
            HelpArgDecl::string("key").with_description("Configuration key to retrieve"),
        ])
        .with_flags(vec![
            HelpFlagDecl::string("source")
                .with_description("Configuration source (local, global, system)")
                .with_default(),
        ]);
    let config = HelpNode::new("config")
        .with_description("Manage application configuration")
        .with_shared_flags(vec![
            HelpFlagDecl::string("profile")
                .with_alias("p")
                .with_description("Configuration profile to use")
                .optional(),
        ])
        .with_subcommands(vec![config_set, config_get]);
    let admin = HelpNode::new("admin")
        .with_description("Administrative commands")
        .with_shared_flags(vec![
            HelpFlagDecl::boolean("sudo")
                .with_description("Run with elevated privileges")
                .with_default(),
        ])
        .with_subcommands(vec![users, config]);
    let copy = HelpNode::new("copy")
        .with_description("Copy files or directories")
        .with_args(vec![
            HelpArgDecl::file("source").with_description("Source file or directory"),
            HelpArgDecl::file("destination").with_description("Destination path"),
        ])
        .with_flags(vec![
            HelpFlagDecl::boolean("recursive")
                .with_alias("r")
                .with_description("Copy directories recursively")
                .with_default(),
            HelpFlagDecl::boolean("force")
                .with_alias("f")
                .with_description("Overwrite existing files")
                .with_default(),
            HelpFlagDecl::integer("buffer-size")
                .with_description("Buffer size in KB")
                .with_default(),
        ]);
    let mov = HelpNode::new("move")
        .with_description("Move or rename files")
        .with_args(vec![
            HelpArgDecl::string("paths")
                .with_description("Source path(s) and destination")
                .variadic_min(2),
        ])
        .with_flags(vec![
            HelpFlagDecl::boolean("interactive")
                .with_alias("i")
                .with_description("Prompt before overwrite")
                .with_default(),
        ]);
    let remove = HelpNode::new("remove")
        .with_description("Remove files or directories")
        .with_args(vec![
            HelpArgDecl::string("files")
                .with_description("Files to remove")
                .variadic_min(1),
        ])
        .with_flags(vec![
            HelpFlagDecl::boolean("recursive")
                .with_alias("r")
                .with_description("Remove directories and contents")
                .with_default(),
            HelpFlagDecl::boolean("force")
                .with_alias("f")
                .with_description("Force removal without prompts")
                .with_default(),
            HelpFlagDecl::boolean("verbose")
                .with_alias("v")
                .with_description("Explain what is being done")
                .with_default(),
        ]);
    HelpNode::new("mycli")
        .with_description("A comprehensive CLI tool demonstrating all features")
        .with_shared_flags(root_shared_flags())
        .with_subcommands(vec![
            admin,
            copy,
            mov,
            remove,
            HelpNode::new("build").with_description("Build the project"),
            HelpNode::new("git").with_description("Git version control"),
            HelpNode::new("test-required").with_description("Test command with required option"),
            HelpNode::new("test-failing").with_description("Test command that always fails"),
            HelpNode::new("app").with_description("Application management"),
            HelpNode::new("app-nested").with_description("Application with nested services"),
        ])
}

/// Render `path` under `root` (the `runCommand(["...", "--help"])` step).
fn render(root: &HelpNode, path: &[&str]) -> String {
    let doc = help_for_path(root, path).expect("path resolves");
    format_help(&doc)
}

#[test]
fn omittable_flags_are_not_required_in_structured_help() {
    // :42 marks omittable flags as not required in structured help
    let node = HelpNode::new("app").with_flags(vec![
        HelpFlagDecl::string("required"),
        HelpFlagDecl::string("optional").optional(),
        HelpFlagDecl::string("defaulted").with_default(),
    ]);
    let doc = node.doc(&["app".to_owned()]);
    let required: Vec<bool> = doc.flags.iter().map(|flag| flag.required).collect();
    assert_eq!(required, vec![true, false, false]);
}

#[test]
fn omittable_arguments_are_not_required_in_structured_help() {
    // :53 marks omittable arguments as not required in structured help
    let required_variadic =
        HelpNode::new("app").with_args(vec![HelpArgDecl::string("files").variadic_min(1)]);
    let optional_variadic =
        HelpNode::new("app").with_args(vec![HelpArgDecl::string("files").variadic()]);
    let defaulted =
        HelpNode::new("app").with_args(vec![HelpArgDecl::string("output").with_default()]);
    assert_eq!(
        [
            required_variadic.doc(&["app".to_owned()]).args[0].required,
            optional_variadic.doc(&["app".to_owned()]).args[0].required,
            defaulted.doc(&["app".to_owned()]).args[0].required,
        ],
        [true, false, false]
    );
}

#[test]
fn renders_root_command_help() {
    // :74 renders root command help
    assert_eq!(render(&comprehensive_mycli(), &["mycli"]), EXPECTED_ROOT);
}

#[test]
fn aligns_flag_descriptions_when_flag_names_are_long() {
    // :111 aligns flag descriptions when flag names are long
    let node = HelpNode::new("tool").with_flags(vec![
        HelpFlagDecl::string("short").with_description("Short flag description"),
        HelpFlagDecl::string("this-is-a-very-very-long-flag-name")
            .with_description("Long flag description"),
    ]);
    let text = render(&node, &["tool"]);
    let lines: Vec<&str> = text.split('\n').collect();
    let short = lines.iter().find(|line| line.contains("--short")).unwrap();
    let long = lines
        .iter()
        .find(|line| line.contains("--this-is-a-very-very-long-flag-name"))
        .unwrap();
    assert_eq!(
        short.find("Short flag description"),
        long.find("Long flag description")
    );
}

#[test]
fn aligns_flag_descriptions_with_wide_graphemes() {
    // :133 aligns flag descriptions when flag names contain wide graphemes
    assert_eq!(visual_width("--ファイル"), 10);
    assert_eq!(visual_width("--👩‍💻"), 4);
    let node = HelpNode::new("tool").with_flags(vec![
        HelpFlagDecl::boolean("ファイル").with_description("Wide flag description"),
        HelpFlagDecl::boolean("👩‍💻").with_description("Emoji flag description"),
        HelpFlagDecl::boolean("verbose").with_description("ASCII flag description"),
    ]);
    let text = render(&node, &["tool"]);
    let lines: Vec<&str> = text.split('\n').collect::<Vec<_>>();
    assert!(lines.contains(&"  --ファイル    Wide flag description"));
    assert!(lines.contains(&"  --👩‍💻          Emoji flag description"));
    assert!(lines.contains(&"  --verbose     ASCII flag description"));
}

#[test]
fn aligns_flag_descriptions_with_zero_width_code_points() {
    // :150 aligns flag descriptions when flag names contain zero-width code points
    assert_eq!(visual_width("--e\u{301}"), 3);
    assert_eq!(visual_width("--a\u{200b}b"), 4);
    let node = HelpNode::new("tool").with_flags(vec![
        HelpFlagDecl::boolean("e\u{301}").with_description("Combining flag description"),
        HelpFlagDecl::boolean("a\u{200b}b").with_description("Zero-width flag description"),
        HelpFlagDecl::boolean("abcd").with_description("ASCII flag description"),
    ]);
    let text = render(&node, &["tool"]);
    let lines: Vec<&str> = text.split('\n').collect::<Vec<_>>();
    assert!(lines.contains(&"  --e\u{301}       Combining flag description"));
    assert!(lines.contains(&"  --a\u{200b}b      Zero-width flag description"));
    assert!(lines.contains(&"  --abcd    ASCII flag description"));
}

#[test]
fn separates_long_subcommand_and_argument_names() {
    // :167 separates long subcommand and argument names from their descriptions
    let child = HelpNode::new("account:set-password")
        .with_description("Rewrite the credential hash")
        .with_args(vec![
            HelpArgDecl::string("existing-account-identifier")
                .with_description("Account to update"),
        ]);
    let root = HelpNode::new("demo").with_subcommands(vec![child]);
    let root_help = render(&root, &["demo"]);
    assert!(
        root_help.contains("account:set-password Rewrite the credential hash"),
        "{root_help}"
    );
    let child_help = render(&root, &["demo", "account:set-password"]);
    assert!(
        child_help.contains("existing-account-identifier string Account to update"),
        "{child_help}"
    );
}

#[test]
fn hides_flags_marked_hidden_from_help_output() {
    // :190 hides flags marked with withHidden from help output
    let node = HelpNode::new("tool").with_flags(vec![
        HelpFlagDecl::string("visible").with_description("Visible flag"),
        HelpFlagDecl::string("experimental-foo")
            .with_description("Should not appear")
            .hidden(),
    ]);
    let text = render(&node, &["tool"]);
    assert!(text.contains("--visible"));
    assert!(!text.contains("experimental-foo"));
    assert!(!text.contains("Should not appear"));
}

#[test]
fn hidden_flag_still_parses_on_the_command_line() {
    // :209 hidden flag still parses on the command line
    let node =
        HelpNode::new("tool").with_flags(vec![HelpFlagDecl::string("experimental-foo").hidden()]);
    assert!(node.find_flag("experimental-foo").is_some());
}

#[test]
fn hidden_flag_name_does_not_leak_through_suggestions() {
    // :225 hidden flag name does not leak through unrecognized-flag suggestions
    let node =
        HelpNode::new("tool").with_flags(vec![HelpFlagDecl::string("experimental-foo").hidden()]);
    let visible = node.visible_flag_names();
    assert!(!visible.contains(&"experimental-foo"));
    let leaked = suggest("experimental-fo", &visible);
    assert!(!leaked.iter().any(|name| name.contains("experimental-foo")));
}

#[test]
fn hides_unlisted_subcommands_from_help_output() {
    // :241 hides unlisted subcommands from help output
    let root = HelpNode::new("tool").with_subcommands(vec![
        HelpNode::new("visible").with_description("A visible subcommand"),
        HelpNode::new("experimental-foo")
            .with_description("Should not appear")
            .unlisted(),
    ]);
    let text = render(&root, &["tool"]);
    assert!(text.contains("visible"));
    assert!(!text.contains("experimental-foo"));
    assert!(!text.contains("Should not appear"));
}

#[test]
fn unlisted_subcommand_still_parses_on_the_command_line() {
    // :263 unlisted subcommand still parses on the command line
    let root =
        HelpNode::new("tool").with_subcommands(vec![HelpNode::new("experimental-foo").unlisted()]);
    assert!(root.find_subcommand("experimental-foo").is_some());
}

#[test]
fn unlisted_subcommand_name_does_not_leak_through_suggestions() {
    // :284 unlisted subcommand name does not leak through unknown-subcommand suggestions
    let root =
        HelpNode::new("tool").with_subcommands(vec![HelpNode::new("experimental-foo").unlisted()]);
    let err = root.resolve_subcommand("experimental-fo").unwrap_err();
    assert!(
        !err.suggestions
            .iter()
            .any(|name| name.contains("experimental-foo"))
    );
}

#[test]
fn subcommand_group_with_only_unlisted_commands_disappears_entirely() {
    // :301 subcommand group with only unlisted commands disappears entirely
    let root =
        HelpNode::new("tool").with_subcommands(vec![HelpNode::new("experimental-foo").unlisted()]);
    let text = render(&root, &["tool"]);
    assert!(!text.contains("SUBCOMMANDS"));
    assert!(!text.contains("<subcommand>"));
}

#[test]
fn renders_command_examples() {
    // :316 renders command examples
    let node = HelpNode::new("login")
        .with_description("Authenticate with Supabase")
        .with_examples(vec![
            HelpExample {
                command: "myapp login".to_owned(),
                description: Some("Log in with browser OAuth".to_owned()),
            },
            HelpExample {
                command: "myapp login --token sbp_abc123".to_owned(),
                description: Some("Log in with a token".to_owned()),
            },
            HelpExample {
                command: "myapp login --logout".to_owned(),
                description: None,
            },
            HelpExample {
                command: "myapp login --logout".to_owned(),
                description: None,
            },
            HelpExample {
                command: "myapp login".to_owned(),
                description: Some("Log in with browser OAuth".to_owned()),
            },
        ]);
    assert_eq!(render(&node, &["login"]), EXPECTED_LOGIN);
}

#[test]
fn renders_file_command_positional_arguments() {
    // :362 renders file command positional arguments
    assert_eq!(
        render(&comprehensive_mycli(), &["mycli", "copy"]),
        EXPECTED_COPY
    );
}

#[test]
fn renders_variadic_arguments() {
    // :394 renders variadic arguments
    assert_eq!(
        render(&comprehensive_mycli(), &["mycli", "remove"]),
        EXPECTED_REMOVE
    );
}

#[test]
fn renders_deeply_nested_subcommand_help() {
    // :425 renders deeply nested subcommand help
    assert_eq!(
        render(&comprehensive_mycli(), &["mycli", "admin", "users", "list"]),
        EXPECTED_LIST
    );
}

#[test]
fn renders_mixed_required_and_optional_positional_arguments() {
    // :454 renders mixed required and optional positional arguments
    assert_eq!(
        render(
            &comprehensive_mycli(),
            &["mycli", "admin", "users", "create"]
        ),
        EXPECTED_CREATE
    );
}

#[test]
fn renders_intermediate_subcommand_shared_flags_and_children() {
    // :486 renders intermediate subcommand shared flags and children
    assert_eq!(
        render(&comprehensive_mycli(), &["mycli", "admin", "config"]),
        EXPECTED_CONFIG
    );
}

#[test]
fn renders_variadic_arguments_with_a_minimum_count() {
    // :517 renders variadic arguments with a minimum count
    assert_eq!(
        render(&comprehensive_mycli(), &["mycli", "admin", "config", "set"]),
        EXPECTED_SET
    );
}

#[test]
fn shared_flags_visible_in_subcommand_help_while_local_flags_stay_local() {
    // :548 shared flags are visible in subcommand help while local flags stay local
    let root = HelpNode::new("tool")
        .with_flags(vec![HelpFlagDecl::string("workspace")])
        .with_shared_flags(vec![HelpFlagDecl::string("model")])
        .with_subcommands(vec![
            HelpNode::new("chat").with_flags(vec![HelpFlagDecl::string("topic")]),
        ]);
    let root_help = render(&root, &["tool"]);
    assert!(root_help.contains("--workspace"));
    assert!(root_help.contains("--model"));
    let chat_help = render(&root, &["tool", "chat"]);
    assert!(!chat_help.contains("--workspace"), "{chat_help}");
    assert!(chat_help.contains("--model"));
    assert!(chat_help.contains("--topic"));
}

#[test]
fn includes_inherited_shared_flags_in_subcommand_completions() {
    // :578 includes inherited shared flags in subcommand completions
    let root = DescriptorCommand::new("tool")
        .with_context_flags(vec![
            ParamDef::flag("workspace", ParamType::String).with_alias("w"),
        ])
        .with_subcommands(vec![DescriptorCommand::new("chat").with_flags(vec![
            ParamDef::flag("topic", ParamType::String).with_alias("t"),
        ])]);
    let script = bash::generate("tool", &from_command(&root, &[]));
    let chat = script.split("_tool_chat()").nth(1).expect("chat function");
    assert!(chat.contains("_filtered_flags+=\" --topic -t\""), "{chat}");
    assert!(
        chat.contains("_filtered_flags+=\" --workspace -w\""),
        "{chat}"
    );
}

#[test]
fn renders_grouped_subcommands() {
    // :600 renders grouped subcommands
    let root = HelpNode::new("tool").with_groups(vec![
        HelpGroup {
            group: Some("Quick Start".to_owned()),
            commands: vec![
                HelpNode::new("init").with_description("Create a new project"),
                HelpNode::new("login").with_description("Authenticate with the platform"),
            ],
        },
        HelpGroup {
            group: Some("Local Development".to_owned()),
            commands: vec![
                HelpNode::new("start").with_description("Start local services"),
                HelpNode::new("stop").with_description("Stop local services"),
                HelpNode::new("db").with_description("Manage local database"),
            ],
        },
        HelpGroup {
            group: Some("Management APIs".to_owned()),
            commands: vec![
                HelpNode::new("projects").with_description("Manage cloud projects"),
                HelpNode::new("functions").with_description("Manage edge functions"),
            ],
        },
        HelpGroup {
            group: None,
            commands: vec![
                HelpNode::new("ungrouped").with_description("This command is not in a group"),
            ],
        },
    ]);
    assert_eq!(render(&root, &["tool"]), EXPECTED_GROUPED);
}

#[test]
fn renders_subcommand_aliases_in_listings() {
    // :665 renders subcommand aliases in listings
    let root = HelpNode::new("tool").with_subcommands(vec![
        HelpNode::new("plan")
            .with_alias("p")
            .with_description("Draft a plan in your editor"),
    ]);
    assert_eq!(render(&root, &["tool"]), EXPECTED_ALIAS);
}

#[test]
fn keeps_subcommand_flags_in_aliased_completion_contexts() {
    // :695 keeps subcommand flags in aliased completion contexts
    let root = DescriptorCommand::new("ctl").with_subcommands(vec![
        DescriptorCommand::new("list")
            .with_alias("ls")
            .with_flags(vec![ParamDef::flag(
                "format",
                ParamType::Choice(vec!["json".to_owned(), "text".to_owned()]),
            )]),
    ]);
    let script = bash::generate("ctl", &from_command(&root, &[]));
    let alias_context = script.split("_ctl_ls()").nth(1).expect("alias function");
    let alias_context = alias_context.split("\n}").next().unwrap_or("");
    assert!(alias_context.contains("--format"), "{alias_context}");
}
const EXPECTED_ROOT: &str = "DESCRIPTION\n  A comprehensive CLI tool demonstrating all features\n\nUSAGE\n  mycli <subcommand> [flags]\n\nFLAGS\n  --debug, -d          Enable debug logging\n  --config, -c file    Path to configuration file\n  --quiet, -q          Suppress non-error output\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)\n\nSUBCOMMANDS\n  admin            Administrative commands\n  copy             Copy files or directories\n  move             Move or rename files\n  remove           Remove files or directories\n  build            Build the project\n  git              Git version control\n  test-required    Test command with required option\n  test-failing     Test command that always fails\n  app              Application management\n  app-nested       Application with nested services";

const EXPECTED_LOGIN: &str = "DESCRIPTION\n  Authenticate with Supabase\n\nUSAGE\n  login [flags]\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)\n\nEXAMPLES\n  # Log in with browser OAuth\n  myapp login\n\n  # Log in with a token\n  myapp login --token sbp_abc123\n\n  myapp login --logout\n  myapp login --logout\n\n  # Log in with browser OAuth\n  myapp login";

const EXPECTED_COPY: &str = "DESCRIPTION\n  Copy files or directories\n\nUSAGE\n  mycli copy [flags] <source> <destination>\n\nARGUMENTS\n  source file         Source file or directory\n  destination file    Destination path\n\nFLAGS\n  --debug, -d              Enable debug logging\n  --config, -c file        Path to configuration file\n  --quiet, -q              Suppress non-error output\n  --recursive, -r          Copy directories recursively\n  --force, -f              Overwrite existing files\n  --buffer-size integer    Buffer size in KB\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)";

const EXPECTED_REMOVE: &str = "DESCRIPTION\n  Remove files or directories\n\nUSAGE\n  mycli remove [flags] <files...>\n\nARGUMENTS\n  files... string    Files to remove\n\nFLAGS\n  --debug, -d          Enable debug logging\n  --config, -c file    Path to configuration file\n  --quiet, -q          Suppress non-error output\n  --recursive, -r      Remove directories and contents\n  --force, -f          Force removal without prompts\n  --verbose, -v        Explain what is being done\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)";

const EXPECTED_LIST: &str = "DESCRIPTION\n  List all users in the system\n\nUSAGE\n  mycli admin users list [flags]\n\nFLAGS\n  --debug, -d          Enable debug logging\n  --config, -c file    Path to configuration file\n  --quiet, -q          Suppress non-error output\n  --sudo               Run with elevated privileges\n  --format string      Output format (json, table, csv)\n  --active             Show only active users\n  --verbose, -v        Show detailed information\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)";

const EXPECTED_CREATE: &str = "DESCRIPTION\n  Create a new user account\n\nUSAGE\n  mycli admin users create [flags] <username> [<email>]\n\nARGUMENTS\n  username string    Username for the new user\n  email string       Email address (optional) (optional)\n\nFLAGS\n  --debug, -d          Enable debug logging\n  --config, -c file    Path to configuration file\n  --quiet, -q          Suppress non-error output\n  --sudo               Run with elevated privileges\n  --role string        User role (admin, user, guest)\n  --notify, -n         Send notification email\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)";

const EXPECTED_CONFIG: &str = "DESCRIPTION\n  Manage application configuration\n\nUSAGE\n  mycli admin config <subcommand> [flags]\n\nFLAGS\n  --debug, -d             Enable debug logging\n  --config, -c file       Path to configuration file\n  --quiet, -q             Suppress non-error output\n  --sudo                  Run with elevated privileges\n  --profile, -p string    Configuration profile to use\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)\n\nSUBCOMMANDS\n  set    Set configuration values\n  get    Get configuration value";

const EXPECTED_SET: &str = "DESCRIPTION\n  Set configuration values\n\nUSAGE\n  mycli admin config set [flags] <key=value...>\n\nARGUMENTS\n  key=value... string    Configuration key-value pairs\n\nFLAGS\n  --debug, -d               Enable debug logging\n  --config, -c file         Path to configuration file\n  --quiet, -q               Suppress non-error output\n  --sudo                    Run with elevated privileges\n  --profile, -p string      Configuration profile to use\n  --config-file, -f file    Write to specific config file\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)";

const EXPECTED_GROUPED: &str = "USAGE\n  tool <subcommand> [flags]\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)\n\nSUBCOMMANDS\n  ungrouped    This command is not in a group\n\nQuick Start:\n  init     Create a new project\n  login    Authenticate with the platform\n\nLocal Development:\n  start    Start local services\n  stop     Stop local services\n  db       Manage local database\n\nManagement APIs:\n  projects     Manage cloud projects\n  functions    Manage edge functions";

const EXPECTED_ALIAS: &str = "USAGE\n  tool <subcommand> [flags]\n\nGLOBAL FLAGS\n  --help, -h                                                          Show help information\n  --version, -v                                                       Show version information\n  --wizard                                                            Start wizard mode for a command\n  --completions <bash|zsh|fish|sh>                                    Print shell completion script (choices: bash, zsh, fish, sh)\n  --log-level <all|trace|debug|info|warn|warning|error|fatal|none>    Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)\n\nSUBCOMMANDS\n  plan, p    Draft a plan in your editor";
