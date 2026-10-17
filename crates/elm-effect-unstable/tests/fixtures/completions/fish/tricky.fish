###-begin-deploy-completions-###
#
# Static completion script for Fish
#
# Installation:
#   deploy --completions fish > ~/.config/fish/completions/deploy.fish
#

complete -c deploy -f
complete -c deploy -n 'begin; not __fish_contains_opt mode; or contains -- (commandline -poc)[-1] --mode; end' -l mode -d 'Deploy mode' -r -f -a 'it\\\'s-fine node\\:20 with\\ space \\(whoami\\) \\#tag \\$HOME back\\\\slash say\\"hi\\" a\\*b a\\;b \\~x foo\\\' a\\!b \\🚀'
complete -c deploy -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt mode' -f -a '--mode' -d 'Deploy mode'
complete -c deploy -r -f -a 'o\\\'clock a\\:b \\{x,y\\} a\\😀b' -d 'Deployment target'

###-end-deploy-completions-###