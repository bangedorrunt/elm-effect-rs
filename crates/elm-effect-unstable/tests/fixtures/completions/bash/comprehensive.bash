###-begin-mycli-completions-###
#
# Static completion script for Bash
#
# Installation:
#   mycli --completions bash >> ~/.bashrc
#

if ! type _init_completion &>/dev/null; then
  _init_completion()
  {
    COMPREPLY=()
    local _i _j=0 _piece _line="$COMP_LINE"
    words=("${COMP_WORDS[0]}")
    cword=0
    _line="${_line#*"${COMP_WORDS[0]}"}"
    for ((_i = 1; _i < ${#COMP_WORDS[@]}; _i++)); do
      _piece="${COMP_WORDS[_i]}"
      if [[ "$_line" == [[:blank:]]* ]]; then
        ((_j++))
        words[_j]="$_piece"
      else
        words[_j]="${words[_j]}$_piece"
      fi
      ((_i == COMP_CWORD)) && cword=$_j
      _line="${_line#*"$_piece"}"
    done
    cur="${words[cword]}"
    prev=""
    ((cword > 0)) && prev="${words[cword-1]}"
    return 0
  }
fi

_mycli--choices()
{
  local _cur="$1" _word="$2"; shift 2

  local _head="${_cur%"$_word"}"
  local _open=""
  case "$_head" in
    *\') _open="'" ;;
    *\") _open='"' ;;
  esac

  local _prefix="$_cur" _committed="$_head"
  _prefix=${_prefix//\\/}; _prefix=${_prefix//\"/}; _prefix=${_prefix//\'/}
  _committed=${_committed//\\/}; _committed=${_committed//\"/}; _committed=${_committed//\'/}

  COMPREPLY=()
  local _choice _rest _match
  for _choice in "$@"; do
    [[ "$_choice" == "$_prefix"* ]] || continue
    _rest="${_choice#"$_committed"}"
    case "$_open" in
      "'")
        if [[ "$_head" == "'" ]]; then
          _match=${_rest//\'/\'\\\'\'}
        else
          [[ "$_rest" == *\'* ]] && continue
          _match="$_rest"
        fi
        ;;
      '"')
        _match="${_rest//\\/\\\\}"
        _match="${_match//\$/\\$}"
        _match="${_match//\`/\\\`}"
        _match="${_match//\"/\\\"}"
        ;;
      *)
        printf -v _match '%q' "$_rest"
        [[ -z "$_head" && "$_match" == '~'* ]] && _match="\\$_match"
        ;;
    esac
    [[ -n "$_open" && "$_match" == *"$_open" ]] && _match+="$_open"
    COMPREPLY+=("$_match")
  done
}

_mycli()
{
  local cur prev words cword i
  local _command_index=0
  _init_completion -n "$COMP_WORDBREAKS" || return
  local _comp_word="$2"

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  # Subcommand dispatch
  local cmd _skip_next=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    case "${words[i]}" in
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      admin)
        _mycli_admin "$i"
        return
        ;;
      copy)
        _mycli_copy "$i"
        return
        ;;
      move)
        _mycli_move "$i"
        return
        ;;
      remove)
        _mycli_remove "$i"
        return
        ;;
      build)
        _mycli_build "$i"
        return
        ;;
      git)
        _mycli_git "$i"
        return
        ;;
      test-required)
        _mycli_test_required "$i"
        return
        ;;
      test-failing)
        _mycli_test_failing "$i"
        return
        ;;
      app)
        _mycli_app "$i"
        return
        ;;
      app-nested)
        _mycli_app_nested "$i"
        return
        ;;
    esac
  done

  local _used_0="" _used_1="" _used_2=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --debug|-d|--no-debug) _used_0=1 ;;
      --config|-c) _used_1=1 ;;
      --quiet|-q|--no-quiet) _used_2=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  COMPREPLY=( $(compgen -W 'admin copy move remove build git test-required test-failing app app-nested' -- "$cur") )
}

_mycli_admin()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  # Subcommand dispatch
  local cmd _skip_next=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    case "${words[i]}" in
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      users)
        _mycli_admin_users "$i"
        return
        ;;
      config)
        _mycli_admin_config "$i"
        return
        ;;
    esac
  done

  local _used_0="" _used_1="" _used_2="" _used_3=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --sudo|--no-sudo) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --sudo --no-sudo"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  COMPREPLY=( $(compgen -W 'users config' -- "$cur") )
}

_mycli_admin_users()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  # Subcommand dispatch
  local cmd _skip_next=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    case "${words[i]}" in
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      list)
        _mycli_admin_users_list "$i"
        return
        ;;
      create)
        _mycli_admin_users_create "$i"
        return
        ;;
    esac
  done

  local _used_0="" _used_1="" _used_2="" _used_3=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --debug|-d|--no-debug) _used_0=1 ;;
      --config|-c) _used_1=1 ;;
      --quiet|-q|--no-quiet) _used_2=1 ;;
      --sudo|--no-sudo) _used_3=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --sudo --no-sudo"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  COMPREPLY=( $(compgen -W 'list create' -- "$cur") )
}

_mycli_admin_users_list()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5="" _used_6=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --format) _used_0=1 ;;
      --active|--no-active) _used_1=1 ;;
      --verbose|-v|--no-verbose) _used_2=1 ;;
      --debug|-d|--no-debug) _used_3=1 ;;
      --config|-c) _used_4=1 ;;
      --quiet|-q|--no-quiet) _used_5=1 ;;
      --sudo|--no-sudo) _used_6=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --format"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --active --no-active"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --verbose -v --no-verbose"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_6" ]] || _filtered_flags+=" --sudo --no-sudo"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_admin_users_create()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --role) _used_0=1 ;;
      --notify|-n|--no-notify) _used_1=1 ;;
      --debug|-d|--no-debug) _used_2=1 ;;
      --config|-c) _used_3=1 ;;
      --quiet|-q|--no-quiet) _used_4=1 ;;
      --sudo|--no-sudo) _used_5=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --role"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --notify -n --no-notify"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --sudo --no-sudo"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_admin_config()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  # Subcommand dispatch
  local cmd _skip_next=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    case "${words[i]}" in
      --profile|-p) _skip_next=1 ;;
      --profile=*|-p=*) ;;
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      set)
        _mycli_admin_config_set "$i"
        return
        ;;
      get)
        _mycli_admin_config_get "$i"
        return
        ;;
    esac
  done

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --profile|-p) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
      --sudo|--no-sudo) _used_4=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --profile -p"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --sudo --no-sudo"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  COMPREPLY=( $(compgen -W 'set get' -- "$cur") )
}

_mycli_admin_config_set()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config-file|-f)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --config-file|-f) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
      --sudo|--no-sudo) _used_4=1 ;;
      --profile|-p) _used_5=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --config-file -f"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --sudo --no-sudo"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --profile -p"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_admin_config_get()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --source) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
      --sudo|--no-sudo) _used_4=1 ;;
      --profile|-p) _used_5=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --source"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --sudo --no-sudo"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --profile -p"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_copy()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --recursive|-r|--no-recursive) _used_0=1 ;;
      --force|-f|--no-force) _used_1=1 ;;
      --buffer-size) _used_2=1 ;;
      --debug|-d|--no-debug) _used_3=1 ;;
      --config|-c) _used_4=1 ;;
      --quiet|-q|--no-quiet) _used_5=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --recursive -r --no-recursive"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --force -f --no-force"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --buffer-size"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  # Positional argument completions
  local _position=0 _skip_next=0 _end_of_options=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    if (( _end_of_options )); then
      ((_position += 1))
      continue
    fi
    case "${words[i]}" in
      --) _end_of_options=1 ;;
      --recursive|-r|--no-recursive) ;;
      --force|-f|--no-force) ;;
      --buffer-size) _skip_next=1 ;;
      --buffer-size=*) ;;
      --debug|-d|--no-debug) ;;
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      --quiet|-q|--no-quiet) ;;
      -*) ;;
      *) ((_position += 1)) ;;
    esac
  done
  case "$_position" in
    0)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
    1)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac
}

_mycli_move()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --interactive|-i|--no-interactive) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --interactive -i --no-interactive"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_remove()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --recursive|-r|--no-recursive) _used_0=1 ;;
      --force|-f|--no-force) _used_1=1 ;;
      --verbose|-v|--no-verbose) _used_2=1 ;;
      --debug|-d|--no-debug) _used_3=1 ;;
      --config|-c) _used_4=1 ;;
      --quiet|-q|--no-quiet) _used_5=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --recursive -r --no-recursive"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --force -f --no-force"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --verbose -v --no-verbose"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_build()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --output|-o) _used_0=1 ;;
      --verbose|-v|--no-verbose) _used_1=1 ;;
      --config-file|-f) _used_2=1 ;;
      --debug|-d|--no-debug) _used_3=1 ;;
      --config|-c) _used_4=1 ;;
      --quiet|-q|--no-quiet) _used_5=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --output -o"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --verbose -v --no-verbose"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config-file -f"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_git()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  # Subcommand dispatch
  local cmd _skip_next=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    case "${words[i]}" in
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      clone)
        _mycli_git_clone "$i"
        return
        ;;
      add)
        _mycli_git_add "$i"
        return
        ;;
      status)
        _mycli_git_status "$i"
        return
        ;;
    esac
  done

  local _used_0="" _used_1="" _used_2="" _used_3=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --verbose|--no-verbose) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --verbose --no-verbose"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  COMPREPLY=( $(compgen -W 'clone add status' -- "$cur") )
}

_mycli_git_clone()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --branch) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
      --verbose|--no-verbose) _used_4=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --branch"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --verbose --no-verbose"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_git_add()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --update|--no-update) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
      --verbose|--no-verbose) _used_4=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --update --no-update"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --verbose --no-verbose"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_git_status()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --short|--no-short) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
      --verbose|--no-verbose) _used_4=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --short --no-short"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --verbose --no-verbose"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_test_required()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --required) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --required"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_test_failing()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --input) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --input"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_app()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  # Subcommand dispatch
  local cmd _skip_next=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    case "${words[i]}" in
      --env) _skip_next=1 ;;
      --env=*) ;;
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      deploy)
        _mycli_app_deploy "$i"
        return
        ;;
    esac
  done

  local _used_0="" _used_1="" _used_2="" _used_3=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --env) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --env"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  COMPREPLY=( $(compgen -W 'deploy' -- "$cur") )
}

_mycli_app_deploy()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5="" _used_6=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --db-host) _used_0=1 ;;
      --db-port) _used_1=1 ;;
      --dry-run|--no-dry-run) _used_2=1 ;;
      --debug|-d|--no-debug) _used_3=1 ;;
      --config|-c) _used_4=1 ;;
      --quiet|-q|--no-quiet) _used_5=1 ;;
      --env) _used_6=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --db-host"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --db-port"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --dry-run --no-dry-run"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_6" ]] || _filtered_flags+=" --env"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

_mycli_app_nested()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  # Subcommand dispatch
  local cmd _skip_next=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    case "${words[i]}" in
      --env) _skip_next=1 ;;
      --env=*) ;;
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      service)
        _mycli_app_nested_service "$i"
        return
        ;;
    esac
  done

  local _used_0="" _used_1="" _used_2="" _used_3=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --env) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --env"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  COMPREPLY=( $(compgen -W 'service' -- "$cur") )
}

_mycli_app_nested_service()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  # Subcommand dispatch
  local cmd _skip_next=0
  for ((i = _command_index + 1; i < cword; i++)); do
    if (( _skip_next )); then
      _skip_next=0
      continue
    fi
    case "${words[i]}" in
      --name) _skip_next=1 ;;
      --name=*) ;;
      --config|-c) _skip_next=1 ;;
      --config=*|-c=*) ;;
      --env) _skip_next=1 ;;
      --env=*) ;;
      deploy)
        _mycli_app_nested_service_deploy "$i"
        return
        ;;
    esac
  done

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --name) _used_0=1 ;;
      --debug|-d|--no-debug) _used_1=1 ;;
      --config|-c) _used_2=1 ;;
      --quiet|-q|--no-quiet) _used_3=1 ;;
      --env) _used_4=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --name"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --env"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

  COMPREPLY=( $(compgen -W 'deploy' -- "$cur") )
}

_mycli_app_nested_service_deploy()
{
  local cur prev words cword i
  local _command_index="$1"
  _init_completion -n "$COMP_WORDBREAKS" || return

  # Flag value completions
  case "$prev" in
    --config|-c)
      COMPREPLY=( $(compgen -f -- "$cur") )
      return
      ;;
  esac

  local _used_0="" _used_1="" _used_2="" _used_3="" _used_4="" _used_5="" _used_6="" _used_7=""
  for ((i = 1; i < cword; i++)); do
    case "${words[i]%%=*}" in
      --db-host) _used_0=1 ;;
      --db-port) _used_1=1 ;;
      --dry-run|--no-dry-run) _used_2=1 ;;
      --debug|-d|--no-debug) _used_3=1 ;;
      --config|-c) _used_4=1 ;;
      --quiet|-q|--no-quiet) _used_5=1 ;;
      --env) _used_6=1 ;;
      --name) _used_7=1 ;;
    esac
  done
  local _filtered_flags=""
  [[ -n "$_used_0" ]] || _filtered_flags+=" --db-host"
  [[ -n "$_used_1" ]] || _filtered_flags+=" --db-port"
  [[ -n "$_used_2" ]] || _filtered_flags+=" --dry-run --no-dry-run"
  [[ -n "$_used_3" ]] || _filtered_flags+=" --debug -d --no-debug"
  [[ -n "$_used_4" ]] || _filtered_flags+=" --config -c"
  [[ -n "$_used_5" ]] || _filtered_flags+=" --quiet -q --no-quiet"
  [[ -n "$_used_6" ]] || _filtered_flags+=" --env"
  [[ -n "$_used_7" ]] || _filtered_flags+=" --name"

  # Complete flags (filtered) and subcommands
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$_filtered_flags" -- "$cur") )
    return
  fi

}

complete -F _mycli mycli
###-end-mycli-completions-###