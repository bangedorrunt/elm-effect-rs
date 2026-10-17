#compdef top
###-begin-top-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   top --completions zsh > ~/.zsh/completions/_top
#   then add ~/.zsh/completions to your fpath
#

_top() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'sub:'
  )

  local -a specs
  specs=(
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
        sub)
          _top_sub
          ;;
      esac
      ;;
  esac
}

_top_sub() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'action:Perform action'
  )

  local -a specs
  specs=(
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
        action)
          _top_sub_action
          ;;
      esac
      ;;
  esac
}

_top_sub_action() {
  local -a specs
  specs=(
    '(--dry-run --no-dry-run)--dry-run[Dry run mode]'
    '(--dry-run --no-dry-run)--no-dry-run[Disable dry-run]'
  )
  _arguments "${specs[@]}"
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _top "$@"
else
  compdef _top top
fi
###-end-top-completions-###