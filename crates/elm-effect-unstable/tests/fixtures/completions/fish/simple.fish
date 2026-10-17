###-begin-greet-completions-###
#
# Static completion script for Fish
#
# Installation:
#   greet --completions fish > ~/.config/fish/completions/greet.fish
#

complete -c greet -f
complete -c greet -n 'not __fish_contains_opt -s l loud no-loud' -l loud -s l -d 'Shout the greeting'
complete -c greet -n 'not __fish_contains_opt -s l loud no-loud' -l no-loud -d 'Disable loud'
complete -c greet -n 'begin; not __fish_contains_opt times; or contains -- (commandline -poc)[-1] --times; end' -l times -d 'Repeat count' -r -f
complete -c greet -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s l loud no-loud' -f -a '--loud' -d 'Shout the greeting'
complete -c greet -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s l loud no-loud' -f -a '--no-loud' -d 'Disable loud'
complete -c greet -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt times' -f -a '--times' -d 'Repeat count'

###-end-greet-completions-###