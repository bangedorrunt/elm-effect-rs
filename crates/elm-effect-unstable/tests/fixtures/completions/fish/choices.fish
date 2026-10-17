###-begin-deploy-completions-###
#
# Static completion script for Fish
#
# Installation:
#   deploy --completions fish > ~/.config/fish/completions/deploy.fish
#

complete -c deploy -f
complete -c deploy -n 'begin; not __fish_contains_opt env; or contains -- (commandline -poc)[-1] --env; end' -l env -d 'Target environment' -r -f -a 'dev staging prod'
complete -c deploy -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt env' -f -a '--env' -d 'Target environment'
complete -c deploy -r -f -a 'us-east eu-west ap-south' -d 'Deployment region'

###-end-deploy-completions-###