###-begin-noop-completions-###
#
# Static completion script for Bash
#
# Installation:
#   noop --completions bash >> ~/.bashrc
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

_noop--choices()
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

_noop()
{
  local cur prev words cword i
  local _command_index=0
  _init_completion -n "$COMP_WORDBREAKS" || return
  local _comp_word="$2"

}

complete -F _noop noop
###-end-noop-completions-###