#compdef greet
###-begin-greet-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   greet --completions zsh > ~/.zsh/completions/_greet
#   then add ~/.zsh/completions to your fpath
#

_greet() {
  local -a specs
  specs=(
    '(--loud -l --no-loud)--loud[Shout the greeting]'
    '(--loud -l --no-loud)-l[Shout the greeting]'
    '(--loud -l --no-loud)--no-loud[Disable loud]'
    '(--times)--times[Repeat count]:integer:'
    ':Name to greet:'
  )
  _arguments "${specs[@]}"
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _greet "$@"
else
  compdef _greet greet
fi
###-end-greet-completions-###