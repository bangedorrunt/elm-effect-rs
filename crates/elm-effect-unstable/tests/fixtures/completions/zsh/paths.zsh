#compdef process
###-begin-process-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   process --completions zsh > ~/.zsh/completions/_process
#   then add ~/.zsh/completions to your fpath
#

_process() {
  local -a specs
  specs=(
    '(--input)--input[Input file]:file:_files'
    '(--output-dir)--output-dir[Output directory]:directory:_directories'
    ':Source file:_files'
  )
  _arguments "${specs[@]}"
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _process "$@"
else
  compdef _process process
fi
###-end-process-completions-###