###-begin-tool-completions-###
#
# Static completion script for Fish
#
# Installation:
#   tool --completions fish > ~/.config/fish/completions/tool.fish
#

complete -c tool -n '__fish_use_subcommand' -f
complete -c tool -n '__fish_use_subcommand' -f -a 'alpha'
complete -c tool -n '__fish_use_subcommand' -f -a 'beta'
complete -c tool -n '__fish_seen_subcommand_from alpha; and not __fish_seen_subcommand_from common' -f
complete -c tool -n '__fish_seen_subcommand_from alpha; and not __fish_seen_subcommand_from common' -f -a 'common'
complete -c tool -n '__fish_seen_subcommand_from alpha; and __fish_seen_subcommand_from common' -f
complete -c tool -n '__fish_seen_subcommand_from alpha; and __fish_seen_subcommand_from common; and not __fish_contains_opt alpha-only no-alpha-only' -l alpha-only
complete -c tool -n '__fish_seen_subcommand_from alpha; and __fish_seen_subcommand_from common; and not __fish_contains_opt alpha-only no-alpha-only' -l no-alpha-only
complete -c tool -n '__fish_seen_subcommand_from alpha; and __fish_seen_subcommand_from common; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt alpha-only no-alpha-only' -f -a '--alpha-only'
complete -c tool -n '__fish_seen_subcommand_from alpha; and __fish_seen_subcommand_from common; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt alpha-only no-alpha-only' -f -a '--no-alpha-only'
complete -c tool -n '__fish_seen_subcommand_from beta; and not __fish_seen_subcommand_from common' -f
complete -c tool -n '__fish_seen_subcommand_from beta; and not __fish_seen_subcommand_from common' -f -a 'common'
complete -c tool -n '__fish_seen_subcommand_from beta; and __fish_seen_subcommand_from common' -f
complete -c tool -n '__fish_seen_subcommand_from beta; and __fish_seen_subcommand_from common; and not __fish_contains_opt beta-only no-beta-only' -l beta-only
complete -c tool -n '__fish_seen_subcommand_from beta; and __fish_seen_subcommand_from common; and not __fish_contains_opt beta-only no-beta-only' -l no-beta-only
complete -c tool -n '__fish_seen_subcommand_from beta; and __fish_seen_subcommand_from common; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt beta-only no-beta-only' -f -a '--beta-only'
complete -c tool -n '__fish_seen_subcommand_from beta; and __fish_seen_subcommand_from common; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt beta-only no-beta-only' -f -a '--no-beta-only'

###-end-tool-completions-###