#compdef deploy
###-begin-deploy-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   deploy --completions zsh > ~/.zsh/completions/_deploy
#   then add ~/.zsh/completions to your fpath
#

_deploy() {
  local -a specs
  specs=(
    '(--mode)--mode[Deploy mode]:value:(it\'\''s-fine node\:20 with\ space \(whoami\) \#tag \$HOME back\\slash say\"hi\" a\*b a\;b \~x foo\'\'' a\!b \🚀)'
    ':Deployment target:(o\'\''clock a\:b \{x,y\} a\😀b)'
  )
  _arguments "${specs[@]}"
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _deploy "$@"
else
  compdef _deploy deploy
fi
###-end-deploy-completions-###