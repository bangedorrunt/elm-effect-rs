###-begin-deploy-completions-###
#
# Static completion script for Fish
#
# Installation:
#   deploy --completions fish > ~/.config/fish/completions/deploy.fish
#

complete -c deploy -f
complete -c deploy -n 'begin; not __fish_contains_opt mode; or contains -- (commandline -poc)[-1] --mode; end' -l mode -d 'Path like C:\\' -r -f -a 'a'
complete -c deploy -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt mode' -f -a '--mode' -d 'Path like C:\\'

###-end-deploy-completions-###