#!/usr/bin/env bash
# Regenerate conformance/modules.toml from a checkout of the pin.
# Usage: conformance/inventory.sh /path/to/effect-repo
# Bash, not POSIX sh: the test index is an associative array.
set -euo pipefail
export LC_ALL=C

if [[ $# -ne 1 ]]; then
  echo "usage: conformance/inventory.sh /path/to/effect-repo" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
checkout="$(cd "$1" && pwd)"
src="$checkout/packages/effect/src"
native_file="$root/conformance/native.txt"
override_file="$root/conformance/overrides.toml"
dest="$root/conformance/modules.toml"

if [[ ! -d $src ]]; then
  echo "missing $src" >&2
  exit 1
fi

declare -A override_class=()
declare -A override_reason=()
if [[ -f $override_file ]]; then
  path=""
  class=""
  reason=""
  flush_override() {
    if [[ -n $path && -n $class ]]; then
      override_class[$path]=$class
      override_reason[$path]=$reason
    fi
  }
  while IFS= read -r line || [[ -n $line ]]; do
    case "$line" in
      "[[override]]")
        flush_override
        path=""
        class=""
        reason=""
        ;;
      path\ =\ \"*\")
        path=${line#*\"}
        path=${path%\"}
        ;;
      class\ =\ \"*\")
        class=${line#*\"}
        class=${class%\"}
        ;;
      reason\ =\ \"*\")
        reason=${line#*\"}
        reason=${reason%\"}
        ;;
    esac
  done <"$override_file"
  flush_override
fi

# Key is the path under src/ or test/ with the extension removed, so
# src/ai/Prompt.ts links only to test/ai/Prompt.test.ts.
declare -A tests_for=()
if [[ -d $checkout/packages/effect/test ]]; then
  while IFS= read -r test_path; do
    rel="${test_path#"$checkout"/}"
    key="${rel#packages/effect/test/}"
    key="${key%.test.ts}"
    if [[ -n ${tests_for[$key]+x} ]]; then
      tests_for[$key]+=$'\n'"$rel"
    else
      tests_for[$key]=$rel
    fi
  done < <(find "$checkout/packages/effect/test" -name '*.test.ts' | sort)
fi

declare -A prev_module_status=()
if [[ -f $dest ]]; then
  prev_path=""
  while IFS= read -r line || [[ -n $line ]]; do
    case "$line" in
      path\ =\ \"*\")
        prev_path=${line#*\"}
        prev_path=${prev_path%\"}
        ;;
      status\ =\ \"*\")
        if [[ -n $prev_path ]]; then
          prev_module_status[$prev_path]=${line#*\"}
          prev_module_status[$prev_path]=${prev_module_status[$prev_path]%\"}
        fi
        ;;
    esac
  done <"$dest"
fi

tag_of() {
  local head
  head="$(head -n 40 "$1")"
  if [[ $head == *"@stability experimental"* ]]; then
    printf '%s' experimental
  elif [[ $head == *"@stability unstable"* ]]; then
    printf '%s' unstable
  else
    printf '%s' none
  fi
}

is_foreign() {
  local in_doc=0 line doc=""
  while IFS= read -r line; do
    if [[ $in_doc -eq 0 ]]; then
      [[ $line =~ ^[[:space:]]*/\*\* ]] || continue
      in_doc=1
    fi
    doc+="$line"$'\n'
    [[ $line == *"*/"* ]] && break
  done < <(head -n 40 "$1")
  [[ $doc == *"fromEventListener"* || $doc == *"platform-node"* || $doc == *"platform-bun"* || $doc == *"DOM event"* ]]
}

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
count=0

{
  while IFS= read -r file; do
    rel="${file#"$checkout"/}"
    tag="$(tag_of "$file")"
    stem="$(basename "$file" .ts)"
    class=""
    status=""
    reason=""
    case "$tag" in
      experimental)
        class="experimental-port"
        status="unmatched"
        ;;
      unstable)
        class="unstable-port"
        status="unmatched"
        ;;
      none)
        if grep -qx "$stem" "$native_file"; then
          class="native"
          status="classified"
          reason="typescript polyfill"
        elif is_foreign "$file"; then
          class="platform-foreign"
          status="classified"
          reason="host adapter"
        else
          class="port"
          status="unmatched"
        fi
        ;;
    esac
    if [[ $tag == none && -n ${override_class[$rel]+x} ]]; then
      class="${override_class[$rel]}"
      reason="${override_reason[$rel]}"
      if [[ $class == native || $class == platform-foreign ]]; then
        status="classified"
      else
        status="unmatched"
      fi
    fi
    if [[ -n ${prev_module_status[$rel]+x} ]]; then
      case "${prev_module_status[$rel]}" in
        matched | unmatched)
          if [[ $class == port || $class == unstable-port || $class == experimental-port ]]; then
            status="${prev_module_status[$rel]}"
          fi
          ;;
      esac
    fi
    src_key="${rel#packages/effect/src/}"
    src_key="${src_key%.ts}"
    tests_field=""
    if [[ -n ${tests_for[$src_key]+x} ]]; then
      while IFS= read -r test_rel; do
        [[ -z $test_rel ]] && continue
        if [[ -n $tests_field ]]; then
          tests_field+=", "
        fi
        tests_field+="\"$test_rel\""
      done < <(printf '%s\n' "${tests_for[$src_key]}" | sort)
    fi
    printf '%s\n' "[[module]]"
    printf 'path = "%s"\n' "$rel"
    printf 'class = "%s"\n' "$class"
    printf 'status = "%s"\n' "$status"
    printf 'tag = "%s"\n' "$tag"
    printf 'reason = "%s"\n' "$reason"
    printf 'tests = [%s]\n' "$tests_field"
    printf '\n'
    count=$((count + 1))
  done < <(find "$src" -name '*.ts' ! -path '*/internal/*' | sort)
} >"$tmp"

{
  printf '%s\n' "# generated by conformance/inventory.sh"
  printf '# modules %s\n' "$count"
  printf '\n'
  cat "$tmp"
} >"$dest"
echo "wrote $count modules to $dest"

# One row per behavior test under packages/effect/test, including suite
# helpers that are not named .test.ts. Status survives a regen when path,
# line, and title are unchanged.
behavior_dest="$root/conformance/behavior-tests.tsv"
declare -A prev_status=()
declare -A prev_rust=()
if [[ -f $behavior_dest ]]; then
  while IFS=$'\t' read -r status path line _kind title rust || [[ -n ${status:-} ]]; do
    [[ $status == \#* || $status == status || -z ${path:-} ]] && continue
    prev_status[$path$'\t'$line$'\t'$title]=$status
    prev_rust[$path$'\t'$line$'\t'$title]=${rust:-}
  done <"$behavior_dest"
fi

behavior_tmp="$(mktemp)"
behavior_count=0
take_title() {
  local chunk="$1"
  if [[ $chunk =~ \"([^\"]*)\" ]]; then
    title="${BASH_REMATCH[1]}"
  elif [[ $chunk =~ \'([^\']*)\' ]]; then
    title="${BASH_REMATCH[1]}"
  elif [[ $chunk =~ \`([^\`]*)\` ]]; then
    title="${BASH_REMATCH[1]}"
  fi
}
while IFS= read -r file; do
  rel="${file#"$checkout"/}"
  mapfile -t lines <"$file"
  for ((i = 0; i < ${#lines[@]}; i++)); do
    line="${lines[$i]}"
    [[ $line =~ ^[[:space:]]*(it\.effect|it\.live|it\.layer|it\.skipIf|it\.skip|it\.each|it|test)\( ]] || continue
    kind="${BASH_REMATCH[1]}"
    title=""
    if [[ $kind == it.each ]]; then
      for ((j = i; j < i + 80 && j < ${#lines[@]}; j++)); do
        chunk="${lines[$j]}"
        if [[ $chunk =~ \)[[:space:]]*\(\"([^\"]*)\" || $chunk =~ \)[[:space:]]*\(\'([^\']*)\' || $chunk =~ \)[[:space:]]*\(\`([^\`]*)\` ]]; then
          title="${BASH_REMATCH[1]}"
          break
        fi
        if [[ $chunk =~ \)[[:space:]]*\([[:space:]]*$ && $((j + 1)) -lt ${#lines[@]} ]]; then
          take_title "${lines[$((j + 1))]}"
          [[ -n $title ]] && break
        fi
      done
    else
      take_title "$line"
      if [[ -z $title ]]; then
        for ((j = i + 1; j < i + 8 && j < ${#lines[@]}; j++)); do
          take_title "${lines[$j]}"
          [[ -n $title ]] && break
        done
      fi
      if [[ -z $title && $line =~ \(([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*, ]]; then
        title="${BASH_REMATCH[1]}"
      fi
    fi
    [[ -z $title ]] && title="untitled"
    title="${title//$'\t'/ }"
    lineno=$((i + 1))
    status="unported"
    [[ $kind == it.skip ]] && status="skipped-upstream"
    key=$rel$'\t'$lineno$'\t'$title
    rust=""
    if [[ -n ${prev_status[$key]+x} ]]; then
      status="${prev_status[$key]}"
      rust="${prev_rust[$key]:-}"
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$status" "$rel" "$lineno" "$kind" "$title" "$rust" >>"$behavior_tmp"
    behavior_count=$((behavior_count + 1))
  done
done < <(find "$checkout/packages/effect/test" -name '*.ts' ! -path '*/internal/*' | sort)

{
  printf '%s\n' "# generated by conformance/inventory.sh"
  printf '# behaviors %s\n' "$behavior_count"
  printf '%s\n' $'status\tpath\tline\tkind\ttitle\trust'
  cat "$behavior_tmp"
} >"$behavior_dest"
rm -f "$behavior_tmp"
echo "wrote $behavior_count behavior tests to $behavior_dest"
