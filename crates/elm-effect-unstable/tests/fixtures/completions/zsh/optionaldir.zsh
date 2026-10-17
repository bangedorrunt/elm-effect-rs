#compdef example
###-begin-example-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   example --completions zsh > ~/.zsh/completions/_example
#   then add ~/.zsh/completions to your fpath
#

_example() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'serve:Start the server'
  )

  local -a specs
  specs=(
    -
    parent-arguments
    ':Directory to start in:_directories'
    -
    subcommands
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        serve)
          _example_serve
          ;;
      esac
      ;;
  esac
}

_example_serve() {
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _example "$@"
else
  compdef _example example
fi
###-end-example-completions-###