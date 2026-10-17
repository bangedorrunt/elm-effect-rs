#compdef noop
###-begin-noop-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   noop --completions zsh > ~/.zsh/completions/_noop
#   then add ~/.zsh/completions to your fpath
#

_noop() {
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _noop "$@"
else
  compdef _noop noop
fi
###-end-noop-completions-###