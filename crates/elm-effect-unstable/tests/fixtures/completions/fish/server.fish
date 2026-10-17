###-begin-server-completions-###
#
# Static completion script for Fish
#
# Installation:
#   server --completions fish > ~/.config/fish/completions/server.fish
#

complete -c server -n '__fish_use_subcommand' -f
complete -c server -n '__fish_use_subcommand' -f -a 'start' -d 'Start the server'
complete -c server -n '__fish_use_subcommand' -f -a 'stop' -d 'Stop the server'
complete -c server -n '__fish_use_subcommand; and not __fish_contains_opt -s v verbose no-verbose' -l verbose -s v
complete -c server -n '__fish_use_subcommand; and not __fish_contains_opt -s v verbose no-verbose' -l no-verbose
complete -c server -n '__fish_use_subcommand; and begin; not __fish_contains_opt config; or contains -- (commandline -poc)[-1] --config; end' -l config -r -f
complete -c server -n '__fish_use_subcommand; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s v verbose no-verbose' -f -a '--verbose'
complete -c server -n '__fish_use_subcommand; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s v verbose no-verbose' -f -a '--no-verbose'
complete -c server -n '__fish_use_subcommand; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt config' -f -a '--config'
complete -c server -n '__fish_seen_subcommand_from start' -f
complete -c server -n '__fish_seen_subcommand_from start; and begin; not __fish_contains_opt -s p port; or contains -- (commandline -poc)[-1] --port -p; end' -l port -s p -d 'Port number' -r -f
complete -c server -n '__fish_seen_subcommand_from start; and not __fish_contains_opt daemon no-daemon' -l daemon -d 'Run as daemon'
complete -c server -n '__fish_seen_subcommand_from start; and not __fish_contains_opt daemon no-daemon' -l no-daemon -d 'Disable daemon'
complete -c server -n '__fish_seen_subcommand_from start; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s p port' -f -a '--port' -d 'Port number'
complete -c server -n '__fish_seen_subcommand_from start; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt daemon no-daemon' -f -a '--daemon' -d 'Run as daemon'
complete -c server -n '__fish_seen_subcommand_from start; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt daemon no-daemon' -f -a '--no-daemon' -d 'Disable daemon'
complete -c server -n '__fish_seen_subcommand_from stop' -f
complete -c server -n '__fish_seen_subcommand_from stop; and not __fish_contains_opt -s f force no-force' -l force -s f -d 'Force stop'
complete -c server -n '__fish_seen_subcommand_from stop; and not __fish_contains_opt -s f force no-force' -l no-force -d 'Disable force'
complete -c server -n '__fish_seen_subcommand_from stop; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s f force no-force' -f -a '--force' -d 'Force stop'
complete -c server -n '__fish_seen_subcommand_from stop; and not string match -q -- "-*" (commandline -ct); and not __fish_contains_opt -s f force no-force' -f -a '--no-force' -d 'Disable force'

###-end-server-completions-###