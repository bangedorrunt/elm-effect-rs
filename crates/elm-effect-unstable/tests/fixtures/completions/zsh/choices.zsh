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
    '(--env)--env[Target environment]:value:(dev staging prod)'
    ':Deployment region:(us-east eu-west ap-south)'
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