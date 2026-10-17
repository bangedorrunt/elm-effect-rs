#compdef server
###-begin-server-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   server --completions zsh > ~/.zsh/completions/_server
#   then add ~/.zsh/completions to your fpath
#

_server() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'start:Start the server'
    'stop:Stop the server'
  )

  local -a specs
  specs=(
    '(--verbose -v --no-verbose)--verbose'
    '(--verbose -v --no-verbose)-v'
    '(--verbose -v --no-verbose)--no-verbose'
    '(--config)--config:value:'
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
        start)
          _server_start
          ;;
        stop)
          _server_stop
          ;;
      esac
      ;;
  esac
}

_server_start() {
  local -a specs
  specs=(
    '(--port -p)--port[Port number]:integer:'
    '(--port -p)-p[Port number]:integer:'
    '(--daemon --no-daemon)--daemon[Run as daemon]'
    '(--daemon --no-daemon)--no-daemon[Disable daemon]'
  )
  _arguments "${specs[@]}"
}

_server_stop() {
  local -a specs
  specs=(
    '(--force -f --no-force)--force[Force stop]'
    '(--force -f --no-force)-f[Force stop]'
    '(--force -f --no-force)--no-force[Disable force]'
  )
  _arguments "${specs[@]}"
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _server "$@"
else
  compdef _server server
fi
###-end-server-completions-###