#compdef mycli
###-begin-mycli-completions-###
#
# Static completion script for Zsh
#
# Installation:
#   mycli --completions zsh > ~/.zsh/completions/_mycli
#   then add ~/.zsh/completions to your fpath
#

_mycli() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'admin:Administrative commands'
    'copy:Copy files or directories'
    'move:Move or rename files'
    'remove:Remove files or directories'
    'build:Build the project'
    'git:Git version control'
    'test-required:Test command with required option'
    'test-failing:Test command that always fails'
    'app:Application management'
    'app-nested:Application with nested services'
  )

  local -a specs
  specs=(
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        admin)
          _mycli_admin
          ;;
        copy)
          _mycli_copy
          ;;
        move)
          _mycli_move
          ;;
        remove)
          _mycli_remove
          ;;
        build)
          _mycli_build
          ;;
        git)
          _mycli_git
          ;;
        test-required)
          _mycli_test_required
          ;;
        test-failing)
          _mycli_test_failing
          ;;
        app)
          _mycli_app
          ;;
        app-nested)
          _mycli_app_nested
          ;;
      esac
      ;;
  esac
}

_mycli_admin() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'users:User management commands'
    'config:Manage application configuration'
  )

  local -a specs
  specs=(
    '(--sudo --no-sudo)--sudo[Run with elevated privileges]'
    '(--sudo --no-sudo)--no-sudo[Disable sudo]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        users)
          _mycli_admin_users
          ;;
        config)
          _mycli_admin_config
          ;;
      esac
      ;;
  esac
}

_mycli_admin_users() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'list:List all users in the system'
    'create:Create a new user account'
  )

  local -a specs
  specs=(
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--sudo --no-sudo)--sudo[Run with elevated privileges]'
    '(--sudo --no-sudo)--no-sudo[Disable sudo]'
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        list)
          _mycli_admin_users_list
          ;;
        create)
          _mycli_admin_users_create
          ;;
      esac
      ;;
  esac
}

_mycli_admin_users_list() {
  local -a specs
  specs=(
    '(--format)--format[Output format (json, table, csv)]:value:'
    '(--active --no-active)--active[Show only active users]'
    '(--active --no-active)--no-active[Disable active]'
    '(--verbose -v --no-verbose)--verbose[Show detailed information]'
    '(--verbose -v --no-verbose)-v[Show detailed information]'
    '(--verbose -v --no-verbose)--no-verbose[Disable verbose]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--sudo --no-sudo)--sudo[Run with elevated privileges]'
    '(--sudo --no-sudo)--no-sudo[Disable sudo]'
  )
  _arguments "${specs[@]}"
}

_mycli_admin_users_create() {
  local -a specs
  specs=(
    '(--role)--role[User role (admin, user, guest)]:value:'
    '(--notify -n --no-notify)--notify[Send notification email]'
    '(--notify -n --no-notify)-n[Send notification email]'
    '(--notify -n --no-notify)--no-notify[Disable notify]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--sudo --no-sudo)--sudo[Run with elevated privileges]'
    '(--sudo --no-sudo)--no-sudo[Disable sudo]'
    ':Username for the new user:'
    ':Email address (optional):'
  )
  _arguments "${specs[@]}"
}

_mycli_admin_config() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'set:Set configuration values'
    'get:Get configuration value'
  )

  local -a specs
  specs=(
    '(--profile -p)--profile[Configuration profile to use]:value:'
    '(--profile -p)-p[Configuration profile to use]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--sudo --no-sudo)--sudo[Run with elevated privileges]'
    '(--sudo --no-sudo)--no-sudo[Disable sudo]'
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        set)
          _mycli_admin_config_set
          ;;
        get)
          _mycli_admin_config_get
          ;;
      esac
      ;;
  esac
}

_mycli_admin_config_set() {
  local -a specs
  specs=(
    '(--config-file -f)--config-file[Write to specific config file]:file:_files'
    '(--config-file -f)-f[Write to specific config file]:file:_files'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--sudo --no-sudo)--sudo[Run with elevated privileges]'
    '(--sudo --no-sudo)--no-sudo[Disable sudo]'
    '(--profile -p)--profile[Configuration profile to use]:value:'
    '(--profile -p)-p[Configuration profile to use]:value:'
    '*:Configuration key-value pairs:'
  )
  _arguments "${specs[@]}"
}

_mycli_admin_config_get() {
  local -a specs
  specs=(
    '(--source)--source[Configuration source (local, global, system)]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--sudo --no-sudo)--sudo[Run with elevated privileges]'
    '(--sudo --no-sudo)--no-sudo[Disable sudo]'
    '(--profile -p)--profile[Configuration profile to use]:value:'
    '(--profile -p)-p[Configuration profile to use]:value:'
    ':Configuration key to retrieve:'
  )
  _arguments "${specs[@]}"
}

_mycli_copy() {
  local -a specs
  specs=(
    '(--recursive -r --no-recursive)--recursive[Copy directories recursively]'
    '(--recursive -r --no-recursive)-r[Copy directories recursively]'
    '(--recursive -r --no-recursive)--no-recursive[Disable recursive]'
    '(--force -f --no-force)--force[Overwrite existing files]'
    '(--force -f --no-force)-f[Overwrite existing files]'
    '(--force -f --no-force)--no-force[Disable force]'
    '(--buffer-size)--buffer-size[Buffer size in KB]:integer:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    ':Source file or directory:_files'
    ':Destination path:_files'
  )
  _arguments "${specs[@]}"
}

_mycli_move() {
  local -a specs
  specs=(
    '(--interactive -i --no-interactive)--interactive[Prompt before overwrite]'
    '(--interactive -i --no-interactive)-i[Prompt before overwrite]'
    '(--interactive -i --no-interactive)--no-interactive[Disable interactive]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '*:Source path(s) and destination:'
  )
  _arguments "${specs[@]}"
}

_mycli_remove() {
  local -a specs
  specs=(
    '(--recursive -r --no-recursive)--recursive[Remove directories and contents]'
    '(--recursive -r --no-recursive)-r[Remove directories and contents]'
    '(--recursive -r --no-recursive)--no-recursive[Disable recursive]'
    '(--force -f --no-force)--force[Force removal without prompts]'
    '(--force -f --no-force)-f[Force removal without prompts]'
    '(--force -f --no-force)--no-force[Disable force]'
    '(--verbose -v --no-verbose)--verbose[Explain what is being done]'
    '(--verbose -v --no-verbose)-v[Explain what is being done]'
    '(--verbose -v --no-verbose)--no-verbose[Disable verbose]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '*:Files to remove:'
  )
  _arguments "${specs[@]}"
}

_mycli_build() {
  local -a specs
  specs=(
    '(--output -o)--output[Output directory]:value:'
    '(--output -o)-o[Output directory]:value:'
    '(--verbose -v --no-verbose)--verbose[Enable verbose output]'
    '(--verbose -v --no-verbose)-v[Enable verbose output]'
    '(--verbose -v --no-verbose)--no-verbose[Disable verbose]'
    '(--config-file -f)--config-file[Configuration file path]:value:'
    '(--config-file -f)-f[Configuration file path]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
  )
  _arguments "${specs[@]}"
}

_mycli_git() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'clone:Clone a repository'
    'add:Add files to staging'
    'status:Show repository status'
  )

  local -a specs
  specs=(
    '(--verbose --no-verbose)--verbose[Enable verbose output]'
    '(--verbose --no-verbose)--no-verbose[Disable verbose]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        clone)
          _mycli_git_clone
          ;;
        add)
          _mycli_git_add
          ;;
        status)
          _mycli_git_status
          ;;
      esac
      ;;
  esac
}

_mycli_git_clone() {
  local -a specs
  specs=(
    '(--branch)--branch[Branch to clone]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--verbose --no-verbose)--verbose[Enable verbose output]'
    '(--verbose --no-verbose)--no-verbose[Disable verbose]'
    ':Repository URL or path:'
  )
  _arguments "${specs[@]}"
}

_mycli_git_add() {
  local -a specs
  specs=(
    '(--update --no-update)--update[Update tracked files]'
    '(--update --no-update)--no-update[Disable update]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--verbose --no-verbose)--verbose[Enable verbose output]'
    '(--verbose --no-verbose)--no-verbose[Disable verbose]'
    ':Files to add:'
  )
  _arguments "${specs[@]}"
}

_mycli_git_status() {
  local -a specs
  specs=(
    '(--short --no-short)--short[Show short format]'
    '(--short --no-short)--no-short[Disable short]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--verbose --no-verbose)--verbose[Enable verbose output]'
    '(--verbose --no-verbose)--no-verbose[Disable verbose]'
  )
  _arguments "${specs[@]}"
}

_mycli_test_required() {
  local -a specs
  specs=(
    '(--required)--required[A required option for testing]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
  )
  _arguments "${specs[@]}"
}

_mycli_test_failing() {
  local -a specs
  specs=(
    '(--input)--input[Input that will cause handler to fail]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
  )
  _arguments "${specs[@]}"
}

_mycli_app() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'deploy:Deploy a service'
  )

  local -a specs
  specs=(
    '(--env)--env[Environment setting]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        deploy)
          _mycli_app_deploy
          ;;
      esac
      ;;
  esac
}

_mycli_app_deploy() {
  local -a specs
  specs=(
    '(--db-host)--db-host[Database host]:value:'
    '(--db-port)--db-port[Database port]:integer:'
    '(--dry-run --no-dry-run)--dry-run[Perform a dry run]'
    '(--dry-run --no-dry-run)--no-dry-run[Disable dry-run]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--env)--env[Environment setting]:value:'
    ':Service to deploy:'
    ':Target environment:'
  )
  _arguments "${specs[@]}"
}

_mycli_app_nested() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'service:Service management'
  )

  local -a specs
  specs=(
    '(--env)--env[Environment setting]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        service)
          _mycli_app_nested_service
          ;;
      esac
      ;;
  esac
}

_mycli_app_nested_service() {
  local context state state_descr line
  typeset -A opt_args

  local -a commands
  commands=(
    'deploy:Deploy a service'
  )

  local -a specs
  specs=(
    '(--name)--name[Service name]:value:'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--env)--env[Environment setting]:value:'
    '1:command:->command'
    '*::arg:->args'
  )

  _arguments -C "${specs[@]}"

  case "$state" in
    command)
      _describe -t commands 'commands' commands
      ;;
    args)
      case "$words[1]" in
        deploy)
          _mycli_app_nested_service_deploy
          ;;
      esac
      ;;
  esac
}

_mycli_app_nested_service_deploy() {
  local -a specs
  specs=(
    '(--db-host)--db-host[Database host]:value:'
    '(--db-port)--db-port[Database port]:integer:'
    '(--dry-run --no-dry-run)--dry-run[Perform a dry run]'
    '(--dry-run --no-dry-run)--no-dry-run[Disable dry-run]'
    '(--debug -d --no-debug)--debug[Enable debug logging]'
    '(--debug -d --no-debug)-d[Enable debug logging]'
    '(--debug -d --no-debug)--no-debug[Disable debug]'
    '(--config -c)--config[Path to configuration file]:file:_files'
    '(--config -c)-c[Path to configuration file]:file:_files'
    '(--quiet -q --no-quiet)--quiet[Suppress non-error output]'
    '(--quiet -q --no-quiet)-q[Suppress non-error output]'
    '(--quiet -q --no-quiet)--no-quiet[Disable quiet]'
    '(--env)--env[Environment setting]:value:'
    '(--name)--name[Service name]:value:'
    ':Service to deploy:'
    ':Target environment:'
  )
  _arguments "${specs[@]}"
}

# Handle both direct invocation and autoload
if [[ "${zsh_eval_context[-1]}" == "loadautofunc" ]]; then
  _mycli "$@"
else
  compdef _mycli mycli
fi
###-end-mycli-completions-###