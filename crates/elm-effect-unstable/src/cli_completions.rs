//! `cli/Completions` port, descriptor half.
//!
//! The pin splits this work three ways: the param/command model, the internal
//! descriptor that flattens a command tree into plain data
//! (`cli/internal/completions/descriptor.ts`), and the shell script generators
//! (`cli/internal/completions/{bash,zsh,fish}.ts`). This module is the
//! descriptor: a [`DescriptorCommand`] tree goes in, a [`CommandDescriptor`]
//! comes out. The generators are the separate `Completions.generate` surface
//! and are not built yet.
//!
//! Only the types that change a completion shape survive the trip: `String` is
//! the fallback for everything the descriptor does not distinguish, and the
//! three file readers all read a file.

use crate::cli::primitive::PathKind;

/// The value shape a flag advertises (`Completions.FlagType`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagType {
    /// A switch, completed as `--flag` and `--no-flag`.
    Boolean,
    /// Free text.
    String,
    /// An integer.
    Int,
    /// A finite number.
    Finite,
    /// A date.
    Date,
    /// One of a fixed set.
    Choice(Vec<String>),
    /// A filesystem path.
    Path(PathKind),
}

/// The value shape an argument advertises (`Completions.ArgumentType`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgumentType {
    /// Free text.
    String,
    /// An integer.
    Int,
    /// A finite number.
    Finite,
    /// A date.
    Date,
    /// One of a fixed set.
    Choice(Vec<String>),
    /// A filesystem path.
    Path(PathKind),
}

/// One flag, ready for completion generation (`Completions.FlagDescriptor`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagDescriptor {
    /// Long name, no dashes.
    pub name: String,
    /// Aliases, no dashes.
    pub aliases: Vec<String>,
    /// Help text, when the flag has one.
    pub description: Option<String>,
    /// The value shape.
    pub flag_type: FlagType,
}

/// One positional, ready for completion generation
/// (`Completions.ArgumentDescriptor`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgumentDescriptor {
    /// Name.
    pub name: String,
    /// Help text, when the argument has one.
    pub description: Option<String>,
    /// The argument must be supplied.
    pub required: bool,
    /// The argument consumes the remaining positionals.
    pub variadic: bool,
    /// The value shape.
    pub argument_type: ArgumentType,
}

/// A command tree, flattened (`Completions.CommandDescriptor`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDescriptor {
    /// Command name.
    pub name: String,
    /// Short description when set, else the description.
    pub description: Option<String>,
    /// Flags, local first then inherited.
    pub flags: Vec<FlagDescriptor>,
    /// Positionals in declaration order.
    pub arguments: Vec<ArgumentDescriptor>,
    /// Reachable subcommands, alias entries included.
    pub subcommands: Vec<CommandDescriptor>,
}

/// The primitive a flag or argument carries. The descriptor reads only the
/// tag, so `FileText`, `FileParse` and `FileSchema` stay distinct here and
/// collapse to a file path on the way out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamType {
    /// Free text.
    String,
    /// A switch.
    Boolean,
    /// An integer.
    Int,
    /// A finite number.
    Finite,
    /// A date.
    Date,
    /// One of a fixed set.
    Choice(Vec<String>),
    /// A filesystem path.
    Path(PathKind),
    /// Read a file as text.
    FileText,
    /// Read a file and parse it.
    FileParse,
    /// Read a file and decode it.
    FileSchema,
}

/// A flag or argument as the descriptor sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamDef {
    /// Bound name.
    pub name: String,
    /// Aliases (`withAlias`).
    pub aliases: Vec<String>,
    /// Help text (`withDescription`).
    pub description: Option<String>,
    /// The value shape.
    pub param_type: ParamType,
    /// Absence is allowed (`optional`); arguments only, flags are never
    /// required in the descriptor.
    pub optional: bool,
    /// Consumes the rest (`variadic`); arguments only.
    pub variadic: bool,
    /// Omitted from completions (`withHidden`); flags only.
    pub hidden: bool,
    /// A display name swap (`withMetavar`). It changes help, never the shape.
    pub metavar: Option<String>,
}

impl ParamDef {
    /// A flag of `param_type` (`Flag.Int`, `Flag.Path`, ...).
    pub fn flag(name: &str, param_type: ParamType) -> Self {
        Self {
            name: name.to_owned(),
            aliases: Vec::new(),
            description: None,
            param_type,
            optional: false,
            variadic: false,
            hidden: false,
            metavar: None,
        }
    }

    /// A positional of `param_type` (`Argument.String`, `Argument.File`, ...).
    pub fn argument(name: &str, param_type: ParamType) -> Self {
        Self::flag(name, param_type)
    }

    /// Add an alias (`withAlias`).
    pub fn with_alias(mut self, alias: &str) -> Self {
        self.aliases.push(alias.to_owned());
        self
    }

    /// Attach help text (`withDescription`).
    pub fn with_description(mut self, text: &str) -> Self {
        self.description = Some(text.to_owned());
        self
    }

    /// Swap the displayed name (`withMetavar`).
    pub fn with_metavar(mut self, text: &str) -> Self {
        self.metavar = Some(text.to_owned());
        self
    }

    /// Hide from completions (`withHidden`).
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Allow absence (`optional`).
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Consume the rest (`variadic`).
    pub fn variadic(mut self) -> Self {
        self.variadic = true;
        self
    }
}

/// A command node (`Command.make` plus its combinators).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescriptorCommand {
    /// Command name.
    pub name: String,
    /// Long description.
    pub description: Option<String>,
    /// One-line description; wins over `description` in parent listings.
    pub short_description: Option<String>,
    /// Flags declared here.
    pub flags: Vec<ParamDef>,
    /// Positionals declared here.
    pub arguments: Vec<ParamDef>,
    /// Child commands.
    pub subcommands: Vec<DescriptorCommand>,
    /// Absent from help and completions (`unlisted`).
    pub unlisted: bool,
    /// Second name the command answers to (`withAlias`).
    pub alias: Option<String>,
    /// Flags a parent shares with this command (`context flags`).
    pub context_flags: Vec<ParamDef>,
}

impl DescriptorCommand {
    /// A command with no flags or arguments (`Command.make`).
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    /// Set the long description (`withDescription`).
    pub fn with_description(mut self, text: &str) -> Self {
        self.description = Some(text.to_owned());
        self
    }

    /// Set the one-line description (`withShortDescription`).
    pub fn with_short_description(mut self, text: &str) -> Self {
        self.short_description = Some(text.to_owned());
        self
    }

    /// Add flags (`Command.make`'s second argument).
    pub fn with_flags(mut self, flags: Vec<ParamDef>) -> Self {
        self.flags.extend(flags);
        self
    }

    /// Add positionals.
    pub fn with_arguments(mut self, arguments: Vec<ParamDef>) -> Self {
        self.arguments.extend(arguments);
        self
    }

    /// Add children (`withSubcommands`).
    pub fn with_subcommands(mut self, subcommands: Vec<DescriptorCommand>) -> Self {
        self.subcommands.extend(subcommands);
        self
    }

    /// Keep the command out of help and completions (`withUnlisted`).
    pub fn unlisted(mut self) -> Self {
        self.unlisted = true;
        self
    }

    /// Add a second name (`withAlias`).
    pub fn with_alias(mut self, alias: &str) -> Self {
        self.alias = Some(alias.to_owned());
        self
    }

    /// Flags this command shares with its children (`withSharedFlags`).
    pub fn with_context_flags(mut self, flags: Vec<ParamDef>) -> Self {
        self.context_flags.extend(flags);
        self
    }
}

fn to_flag_type(param_type: &ParamType) -> FlagType {
    match param_type {
        ParamType::Boolean => FlagType::Boolean,
        ParamType::Int => FlagType::Int,
        ParamType::Finite => FlagType::Finite,
        ParamType::Date => FlagType::Date,
        ParamType::Choice(values) => FlagType::Choice(values.clone()),
        ParamType::Path(kind) => FlagType::Path(*kind),
        ParamType::FileText | ParamType::FileParse | ParamType::FileSchema => {
            FlagType::Path(PathKind::File)
        }
        ParamType::String => FlagType::String,
    }
}

fn to_argument_type(param_type: &ParamType) -> ArgumentType {
    match param_type {
        ParamType::Int => ArgumentType::Int,
        ParamType::Finite => ArgumentType::Finite,
        ParamType::Date => ArgumentType::Date,
        ParamType::Choice(values) => ArgumentType::Choice(values.clone()),
        ParamType::Path(kind) => ArgumentType::Path(*kind),
        ParamType::FileText | ParamType::FileParse | ParamType::FileSchema => {
            ArgumentType::Path(PathKind::File)
        }
        ParamType::String | ParamType::Boolean => ArgumentType::String,
    }
}

/// Flatten a command tree for completion generation (`fromCommand`).
///
/// Shape follows help: local flags first, then inherited ones root to leaf,
/// with an inherited name dropped when a local flag already claimed it.
/// Hidden flags and unlisted subcommands never reach the descriptor, so the
/// shell does not advertise what `--help` omits.
pub fn from_command(
    command: &DescriptorCommand,
    inherited_flags: &[ParamDef],
) -> CommandDescriptor {
    let mut flags = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let local = command.flags.len();
    for (index, flag) in command.flags.iter().chain(inherited_flags).enumerate() {
        if flag.hidden {
            continue;
        }
        let inherited = index >= local;
        if inherited && seen.contains(&flag.name.as_str()) {
            continue;
        }
        seen.push(&flag.name);
        flags.push(FlagDescriptor {
            name: flag.name.clone(),
            aliases: flag.aliases.clone(),
            description: flag.description.clone(),
            flag_type: to_flag_type(&flag.param_type),
        });
    }

    let arguments = command
        .arguments
        .iter()
        .map(|argument| ArgumentDescriptor {
            name: argument.name.clone(),
            description: argument.description.clone(),
            required: !argument.optional,
            variadic: argument.variadic,
            argument_type: to_argument_type(&argument.param_type),
        })
        .collect();

    let shared: Vec<ParamDef> = inherited_flags
        .iter()
        .chain(command.context_flags.iter())
        .cloned()
        .collect();
    let mut subcommands = Vec::new();
    for child in &command.subcommands {
        if child.unlisted {
            continue;
        }
        let descriptor = from_command(child, &shared);
        subcommands.push(descriptor.clone());
        if let Some(alias) = &child.alias {
            if alias != &child.name {
                subcommands.push(CommandDescriptor {
                    name: alias.clone(),
                    ..descriptor
                });
            }
        }
    }

    CommandDescriptor {
        name: command.name.clone(),
        description: command
            .short_description
            .clone()
            .or_else(|| command.description.clone()),
        flags,
        arguments,
        subcommands,
    }
}

/// Static Bash completion script generation
/// (`cli/internal/completions/bash.ts`).
///
/// Emits a self-contained script from a [`CommandDescriptor`]: no re-invocation
/// of the CLI at runtime. Stays on Bash 3.2 syntax, which is what macOS ships.
pub mod bash {
    use super::{ArgumentDescriptor, ArgumentType, CommandDescriptor, FlagDescriptor, FlagType};
    use crate::cli::primitive::PathKind;

    /// The inline `_init_completion` fallback for environments without
    /// bash-completion. Held verbatim: every byte is output.
    const INIT_COMPLETION_FALLBACK: &str = r##"if ! type _init_completion &>/dev/null; then
  _init_completion()
  {
    COMPREPLY=()
    local _i _j=0 _piece _line="$COMP_LINE"
    words=("${COMP_WORDS[0]}")
    cword=0
    _line="${_line#*"${COMP_WORDS[0]}"}"
    for ((_i = 1; _i < ${#COMP_WORDS[@]}; _i++)); do
      _piece="${COMP_WORDS[_i]}"
      if [[ "$_line" == [[:blank:]]* ]]; then
        ((_j++))
        words[_j]="$_piece"
      else
        words[_j]="${words[_j]}$_piece"
      fi
      ((_i == COMP_CWORD)) && cword=$_j
      _line="${_line#*"$_piece"}"
    done
    cur="${words[cword]}"
    prev=""
    ((cword > 0)) && prev="${words[cword-1]}"
    return 0
  }
fi"##;

    /// The choice-completion helper body. `compgen -W` reparses quoted values,
    /// so choices are completed by hand, escaping only the part readline
    /// replaces for the active quote context. Held verbatim.
    const CHOICES_HELPER_BODY: &str = r##"{
  local _cur="$1" _word="$2"; shift 2

  local _head="${_cur%"$_word"}"
  local _open=""
  case "$_head" in
    *\') _open="'" ;;
    *\") _open='"' ;;
  esac

  local _prefix="$_cur" _committed="$_head"
  _prefix=${_prefix//\\/}; _prefix=${_prefix//\"/}; _prefix=${_prefix//\'/}
  _committed=${_committed//\\/}; _committed=${_committed//\"/}; _committed=${_committed//\'/}

  COMPREPLY=()
  local _choice _rest _match
  for _choice in "$@"; do
    [[ "$_choice" == "$_prefix"* ]] || continue
    _rest="${_choice#"$_committed"}"
    case "$_open" in
      "'")
        if [[ "$_head" == "'" ]]; then
          _match=${_rest//\'/\'\\\'\'}
        else
          [[ "$_rest" == *\'* ]] && continue
          _match="$_rest"
        fi
        ;;
      '"')
        _match="${_rest//\\/\\\\}"
        _match="${_match//\$/\\$}"
        _match="${_match//\`/\\\`}"
        _match="${_match//\"/\\\"}"
        ;;
      *)
        printf -v _match '%q' "$_rest"
        [[ -z "$_head" && "$_match" == '~'* ]] && _match="\\$_match"
        ;;
    esac
    [[ -n "$_open" && "$_match" == *"$_open" ]] && _match+="$_open"
    COMPREPLY+=("$_match")
  done
}"##;

    /// Close a single-quoted Bash string: `'` becomes `'\''`.
    fn escape_for_bash(text: &str) -> String {
        text.replace('\'', "'\\''")
    }

    /// Function names keep word characters only. A non-ASCII char becomes one
    /// underscore per UTF-16 unit, matching the pin's `replace` over a JS
    /// string (an astral char is two units, so two underscores).
    fn sanitize_function_name(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                out.push(c);
            } else {
                for _ in 0..c.len_utf16() {
                    out.push('_');
                }
            }
        }
        out
    }

    fn function_name(path: &[String]) -> String {
        format!(
            "_{}",
            path.iter()
                .map(|part| sanitize_function_name(part))
                .collect::<Vec<_>>()
                .join("_")
        )
    }

    fn alias_form(alias: &str) -> String {
        if alias.chars().count() == 1 {
            format!("-{alias}")
        } else {
            format!("--{alias}")
        }
    }

    /// Every spelling a flag answers to, for the `case` word list.
    fn flag_names_for_wordlist(flag: &FlagDescriptor) -> Vec<String> {
        let mut names = vec![format!("--{}", flag.name)];
        for alias in &flag.aliases {
            names.push(alias_form(alias));
        }
        if flag.flag_type == FlagType::Boolean {
            names.push(format!("--no-{}", flag.name));
        }
        names
    }

    /// Emit the Bash 3.2-compatible used-flag filter. A flag disappears from
    /// the word list once any of its spellings was typed.
    fn build_flag_group_declarations(flags: &[FlagDescriptor], lines: &mut Vec<String>) {
        if flags.is_empty() {
            return;
        }
        let groups: Vec<Vec<String>> = flags.iter().map(flag_names_for_wordlist).collect();
        let declarations = (0..groups.len())
            .map(|index| format!("_used_{index}=\"\""))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("  local {declarations}"));
        lines.push("  for ((i = 1; i < cword; i++)); do".to_owned());
        lines.push("    case \"${words[i]%%=*}\" in".to_owned());
        for (index, forms) in groups.iter().enumerate() {
            lines.push(format!("      {}) _used_{index}=1 ;;", forms.join("|")));
        }
        lines.push("    esac".to_owned());
        lines.push("  done".to_owned());
        lines.push("  local _filtered_flags=\"\"".to_owned());
        for (index, forms) in groups.iter().enumerate() {
            lines.push(format!(
                "  [[ -n \"$_used_{index}\" ]] || _filtered_flags+=\" {}\"",
                forms.join(" ")
            ));
        }
        lines.push(String::new());
    }

    fn choice_completion(helper_name: &str, values: &[String]) -> String {
        let joined = values
            .iter()
            .map(|value| format!("'{}'", escape_for_bash(value)))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{helper_name} \"$cur\" \"$_comp_word\" {joined}")
    }

    fn flag_value_completion(flag_type: &FlagType, helper_name: &str) -> Option<String> {
        match flag_type {
            FlagType::Boolean => None,
            FlagType::Choice(values) => Some(choice_completion(helper_name, values)),
            FlagType::Path(PathKind::Directory) => {
                Some("COMPREPLY=( $(compgen -d -- \"$cur\") )".to_owned())
            }
            FlagType::Path(_) => Some("COMPREPLY=( $(compgen -f -- \"$cur\") )".to_owned()),
            _ => None,
        }
    }

    fn arg_completion(argument_type: &ArgumentType, helper_name: &str) -> Option<String> {
        match argument_type {
            ArgumentType::Choice(values) => Some(choice_completion(helper_name, values)),
            ArgumentType::Path(PathKind::Directory) => {
                Some("COMPREPLY=( $(compgen -d -- \"$cur\") )".to_owned())
            }
            ArgumentType::Path(_) => Some("COMPREPLY=( $(compgen -f -- \"$cur\") )".to_owned()),
            _ => None,
        }
    }

    fn generate_function(
        descriptor: &CommandDescriptor,
        parent_path: &[String],
        lines: &mut Vec<String>,
        helper_name: &str,
    ) {
        let mut current_path = parent_path.to_vec();
        current_path.push(descriptor.name.clone());
        let func_name = function_name(&current_path);

        lines.push(format!("{func_name}()"));
        lines.push("{".to_owned());
        lines.push("  local cur prev words cword i".to_owned());
        lines.push(if parent_path.is_empty() {
            "  local _command_index=0".to_owned()
        } else {
            "  local _command_index=\"$1\"".to_owned()
        });
        // Keeps values holding COMP_WORDBREAKS characters in one word.
        lines.push("  _init_completion -n \"$COMP_WORDBREAKS\" || return".to_owned());
        if parent_path.is_empty() {
            // Subcommand functions inherit this through Bash's dynamic scope.
            lines.push("  local _comp_word=\"$2\"".to_owned());
        }
        lines.push(String::new());

        let flags_with_values: Vec<&FlagDescriptor> = descriptor
            .flags
            .iter()
            .filter(|flag| flag.flag_type != FlagType::Boolean)
            .collect();
        if !flags_with_values.is_empty() {
            lines.push("  # Flag value completions".to_owned());
            lines.push("  case \"$prev\" in".to_owned());
            for flag in &flags_with_values {
                let mut long_names = vec![format!("--{}", flag.name)];
                for alias in &flag.aliases {
                    long_names.push(alias_form(alias));
                }
                if let Some(completion) = flag_value_completion(&flag.flag_type, helper_name) {
                    lines.push(format!("    {})", long_names.join("|")));
                    lines.push(format!("      {completion}"));
                    lines.push("      return".to_owned());
                    lines.push("      ;;".to_owned());
                }
            }
            lines.push("  esac".to_owned());
            lines.push(String::new());
        }

        if !descriptor.subcommands.is_empty() {
            lines.push("  # Subcommand dispatch".to_owned());
            lines.push("  local cmd _skip_next=0".to_owned());
            lines.push("  for ((i = _command_index + 1; i < cword; i++)); do".to_owned());
            lines.push("    if (( _skip_next )); then".to_owned());
            lines.push("      _skip_next=0".to_owned());
            lines.push("      continue".to_owned());
            lines.push("    fi".to_owned());
            lines.push("    case \"${words[i]}\" in".to_owned());
            for flag in &descriptor.flags {
                if flag.flag_type == FlagType::Boolean {
                    continue;
                }
                let forms = flag_names_for_wordlist(flag);
                lines.push(format!("      {}) _skip_next=1 ;;", forms.join("|")));
                lines.push(format!(
                    "      {}) ;;",
                    forms
                        .iter()
                        .map(|form| format!("{form}=*"))
                        .collect::<Vec<_>>()
                        .join("|")
                ));
            }
            for sub in &descriptor.subcommands {
                let mut sub_path = current_path.clone();
                sub_path.push(sub.name.clone());
                lines.push(format!("      {})", sub.name));
                lines.push(format!("        {} \"$i\"", function_name(&sub_path)));
                lines.push("        return".to_owned());
                lines.push("        ;;".to_owned());
            }
            lines.push("    esac".to_owned());
            lines.push("  done".to_owned());
            lines.push(String::new());
        }

        build_flag_group_declarations(&descriptor.flags, lines);

        if !descriptor.flags.is_empty() || !descriptor.subcommands.is_empty() {
            lines.push("  # Complete flags (filtered) and subcommands".to_owned());
            lines.push("  if [[ \"$cur\" == -* ]]; then".to_owned());
            if !descriptor.flags.is_empty() {
                lines.push(
                    "    COMPREPLY=( $(compgen -W \"$_filtered_flags\" -- \"$cur\") )".to_owned(),
                );
            }
            lines.push("    return".to_owned());
            lines.push("  fi".to_owned());
            lines.push(String::new());
        }

        let args_with_completions: Vec<(usize, &ArgumentDescriptor, String)> = descriptor
            .arguments
            .iter()
            .enumerate()
            .filter_map(|(index, argument)| {
                arg_completion(&argument.argument_type, helper_name)
                    .map(|completion| (index, argument, completion))
            })
            .collect();
        if !args_with_completions.is_empty() {
            lines.push("  # Positional argument completions".to_owned());
            lines.push("  local _position=0 _skip_next=0 _end_of_options=0".to_owned());
            lines.push("  for ((i = _command_index + 1; i < cword; i++)); do".to_owned());
            lines.push("    if (( _skip_next )); then".to_owned());
            lines.push("      _skip_next=0".to_owned());
            lines.push("      continue".to_owned());
            lines.push("    fi".to_owned());
            lines.push("    if (( _end_of_options )); then".to_owned());
            lines.push("      ((_position += 1))".to_owned());
            lines.push("      continue".to_owned());
            lines.push("    fi".to_owned());
            lines.push("    case \"${words[i]}\" in".to_owned());
            lines.push("      --) _end_of_options=1 ;;".to_owned());
            for flag in &descriptor.flags {
                let forms = flag_names_for_wordlist(flag);
                if flag.flag_type == FlagType::Boolean {
                    lines.push(format!("      {}) ;;", forms.join("|")));
                } else {
                    lines.push(format!("      {}) _skip_next=1 ;;", forms.join("|")));
                    lines.push(format!(
                        "      {}) ;;",
                        forms
                            .iter()
                            .map(|form| format!("{form}=*"))
                            .collect::<Vec<_>>()
                            .join("|")
                    ));
                }
            }
            lines.push("      -*) ;;".to_owned());
            lines.push("      *) ((_position += 1)) ;;".to_owned());
            lines.push("    esac".to_owned());
            lines.push("  done".to_owned());
            lines.push("  case \"$_position\" in".to_owned());
            for (index, argument, completion) in &args_with_completions {
                if argument.variadic {
                    continue;
                }
                lines.push(format!("    {index})"));
                lines.push(format!("      {completion}"));
                lines.push("      return".to_owned());
                lines.push("      ;;".to_owned());
            }
            lines.push("  esac".to_owned());
            if let Some((index, _, completion)) =
                args_with_completions.iter().find(|(_, argument, _)| argument.variadic)
            {
                lines.push(format!("  if (( _position >= {index} )); then"));
                lines.push(format!("    {completion}"));
                lines.push("    return".to_owned());
                lines.push("  fi".to_owned());
            }
        } else if !descriptor.subcommands.is_empty() {
            let names = descriptor
                .subcommands
                .iter()
                .map(|sub| sub.name.clone())
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(format!("  COMPREPLY=( $(compgen -W '{names}' -- \"$cur\") )"));
        }

        lines.push("}".to_owned());
        lines.push(String::new());

        for sub in &descriptor.subcommands {
            generate_function(sub, &current_path, lines, helper_name);
        }
    }

    /// Generate the completion script for `executable_name`.
    pub fn generate(executable_name: &str, descriptor: &CommandDescriptor) -> String {
        let mut lines: Vec<String> = Vec::new();
        let safe_name = sanitize_function_name(executable_name);
        // Sanitized names cannot contain `-`, so the helper name is unambiguous.
        let helper_name = format!("_{safe_name}--choices");
        let escaped = escape_for_bash(executable_name);

        lines.push(format!("###-begin-{escaped}-completions-###"));
        lines.push("#".to_owned());
        lines.push("# Static completion script for Bash".to_owned());
        lines.push("#".to_owned());
        lines.push("# Installation:".to_owned());
        lines.push(format!("#   {escaped} --completions bash >> ~/.bashrc"));
        lines.push("#".to_owned());
        lines.push(String::new());

        for line in INIT_COMPLETION_FALLBACK.lines() {
            lines.push(line.to_owned());
        }
        lines.push(String::new());

        lines.push(format!("{helper_name}()"));
        for line in CHOICES_HELPER_BODY.lines() {
            lines.push(line.to_owned());
        }
        lines.push(String::new());

        generate_function(descriptor, &[], &mut lines, &helper_name);

        lines.push(format!("complete -F _{safe_name} {escaped}"));
        lines.push(format!("###-end-{escaped}-completions-###"));

        lines.join("\n")
    }
}
