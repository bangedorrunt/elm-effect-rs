#compdef tool
###-begin-tool-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   tool --completions zsh > ~/.zsh/completions/_tool
#   then add ~/.zsh/completions to your fpath
#

_tool() {
  local -a specs
  specs=(
    '(--verbose -v --no-verbose)--verbose'
    '(--verbose -v --no-verbose)-v'
    '(--verbose -v --no-verbose)--no-verbose'
    '(--format -f)--format:value:(json text)'
    '(--format -f)-f:value:(json text)'
    ':source:(one)'
    ':target:(two)'
  )
  _arguments "${specs[@]}"
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _tool "$@"
else
  compdef _tool tool
fi
###-end-tool-completions-###