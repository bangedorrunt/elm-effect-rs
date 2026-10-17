###-begin-tool-completions-###
#
# Static completion script for Fish
#
# Installation:
#   tool --completions fish > ~/.config/fish/completions/tool.fish
#

complete -c tool -f
complete -c tool -n 'not __fish_contains_opt -s v verbose no-verbose' -l verbose -s v
complete -c tool -n 'not __fish_contains_opt -s v verbose no-verbose' -l no-verbose
complete -c tool -n 'begin; not __fish_contains_opt -s f format; or contains -- (commandline -poc)[-1] --format -f; end' -l format -s f -r -f -a 'json text'
complete -c tool -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s v verbose no-verbose' -f -a '--verbose'
complete -c tool -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s v verbose no-verbose' -f -a '--no-verbose'
complete -c tool -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s f format' -f -a '--format'
complete -c tool -r -f -a 'one'
complete -c tool -r -f -a 'two'

###-end-tool-completions-###