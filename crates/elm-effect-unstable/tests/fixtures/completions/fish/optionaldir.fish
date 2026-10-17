###-begin-example-completions-###
#
# Static completion script for Fish
#
# Installation:
#   example --completions fish > ~/.config/fish/completions/example.fish
#

complete -c example -n '__fish_use_subcommand' -f -a 'serve' -d 'Start the server'
complete -c example -n '__fish_use_subcommand' -r -F -d 'Directory to start in'
complete -c example -n '__fish_seen_subcommand_from serve' -f

###-end-example-completions-###