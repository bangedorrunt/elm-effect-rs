###-begin-process-completions-###
#
# Static completion script for Fish
#
# Installation:
#   process --completions fish > ~/.config/fish/completions/process.fish
#

complete -c process -n 'begin; not __fish_contains_opt input; or contains -- (commandline -poc)[-1] --input; end' -l input -d 'Input file' -r -F
complete -c process -n 'begin; not __fish_contains_opt output-dir; or contains -- (commandline -poc)[-1] --output-dir; end' -l output-dir -d 'Output directory' -r -F
complete -c process -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt input' -f -a '--input' -d 'Input file'
complete -c process -n 'not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt output-dir' -f -a '--output-dir' -d 'Output directory'
complete -c process -r -F -d 'Source file'

###-end-process-completions-###