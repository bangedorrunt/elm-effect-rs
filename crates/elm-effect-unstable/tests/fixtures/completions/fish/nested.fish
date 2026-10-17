###-begin-top-completions-###
#
# Static completion script for Fish
#
# Installation:
#   top --completions fish > ~/.config/fish/completions/top.fish
#

complete -c top -n '__fish_use_subcommand' -f
complete -c top -n '__fish_use_subcommand' -f -a 'sub'
complete -c top -n '__fish_seen_subcommand_from sub; and not __fish_seen_subcommand_from action' -f
complete -c top -n '__fish_seen_subcommand_from sub; and not __fish_seen_subcommand_from action' -f -a 'action' -d 'Perform action'
complete -c top -n '__fish_seen_subcommand_from sub; and __fish_seen_subcommand_from action' -f
complete -c top -n '__fish_seen_subcommand_from sub; and __fish_seen_subcommand_from action; and not __fish_contains_opt dry-run no-dry-run' -l dry-run -d 'Dry run mode'
complete -c top -n '__fish_seen_subcommand_from sub; and __fish_seen_subcommand_from action; and not __fish_contains_opt dry-run no-dry-run' -l no-dry-run -d 'Disable dry-run'
complete -c top -n '__fish_seen_subcommand_from sub; and __fish_seen_subcommand_from action; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt dry-run no-dry-run' -f -a '--dry-run' -d 'Dry run mode'
complete -c top -n '__fish_seen_subcommand_from sub; and __fish_seen_subcommand_from action; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt dry-run no-dry-run' -f -a '--no-dry-run' -d 'Disable dry-run'

###-end-top-completions-###