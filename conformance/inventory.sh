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
declare -A tests_by_base=()
if [[ -d $checkout/packages/effect/test ]]; then
  while IFS= read -r test_path; do
    rel="${test_path#"$checkout"/}"
    key="${rel#packages/effect/test/}"
    # Two naming conventions live side by side: `Foo.test.ts` and, for the
    # persistence and MCP conformance suites, `FooTest.ts` / `Foo.test-utils.ts`.
    # The behavior scanner below reads every `.ts`, so these rows are already
    # witnessed; only the link needs the wider match, or their module reads
    # `tests = []` while its rows sit in the ledger.
    case "$key" in
      *.test.ts)    key="${key%.test.ts}" ;;
      *Test.ts)     key="${key%Test.ts}" ;;
      *) continue ;;
    esac
    if [[ -n ${tests_for[$key]+x} ]]; then
      tests_for[$key]+=$'\n'"$rel"
    else
      tests_for[$key]=$rel
    fi
    # The Effect repo does not lay src/ and test/ out symmetrically, so a
    # module can be tested from a directory its own path does not predict:
    # src/Schema.ts by test/schema/Schema.test.ts, src/testing/TestClock.ts by
    # test/TestClock.test.ts. The relative key misses both, so index the
    # basename as a second, weaker route (see the fallback below).
    base="${rel##*/}"
    base="${base%.test.ts}"
    base="${base%Test.ts}"
    if [[ -n ${tests_by_base[$base]+x} ]]; then
      tests_by_base[$base]+=$'\n'"$rel"
    else
      tests_by_base[$base]=$rel
    fi
  done < <(find "$checkout/packages/effect/test" -name '*.ts' ! -path '*/internal/*' | sort)
fi

# Which test files a module already links by the relative key. A basename is
# not unique across the package (test/Graph.test.ts and
# test/schema/Graph.test.ts both exist), and a test file can be claimed by a
# module the basename does not name: test/ai/Model.test.ts belongs to
# src/ai/Model.ts, so src/schema/Model.ts must not fall back onto it. The
# relative key is the stronger claim, so collect them before any fallback.
declare -A claimed_by_relative=()
while IFS= read -r file; do
  rel="${file#"$checkout"/}"
  src_key="${rel#packages/effect/src/}"
  src_key="${src_key%.ts}"
  [[ -z ${tests_for[$src_key]+x} ]] && continue
  while IFS= read -r t; do
    [[ -z $t ]] && continue
    claimed_by_relative[$t]=1
  done < <(printf '%s\n' "${tests_for[$src_key]}")
done < <(find "$src" -name '*.ts' ! -path '*/internal/*' | sort)

declare -A prev_module_status=()
declare -A prev_module_reason=()
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
      reason\ =\ \"*\")
        if [[ -n $prev_path ]]; then
          prev_module_reason[$prev_path]=${line#*\"}
          prev_module_reason[$prev_path]=${prev_module_reason[$prev_path]%\"}
        fi
        ;;
    esac
  done <"$dest"
fi

# ADR-0002 '"'"'Behavior parity'"'"': "A module row is not `matched` while any
# behavior row from its linked test file is still `unported`." That rule lived
# only in prose, so a hand-set `matched` survived every regen: 26 modules
# claimed parity with up to 122 of their own rows still unported, and 31 more
# were `matched` with no linked test file at all. The status is derived here
# rather than carried, so a regen cannot drift.
#
# Read from the previous behavior file rather than from the one this run
# writes: a row'"'"'s status only changes when its (path, line, title) key
# changes, which is itself a regeneration artifact, so the previous file'"'"'s
# counts are the counts this run will emit.
declare -A rows_in=()
declare -A ported_in=()
declare -A unported_in=()
behavior_prev="$root/conformance/behavior-tests.tsv"
if [[ -f $behavior_prev ]]; then
  while IFS=$'\t' read -r bstatus bpath _bline _bkind _btitle _brust || [[ -n ${bstatus:-} ]]; do
    [[ $bstatus == \#* || $bstatus == status || -z ${bpath:-} ]] && continue
    rows_in[$bpath]=$(( ${rows_in[$bpath]:-0} + 1 ))
    case "$bstatus" in
      unported) unported_in[$bpath]=$(( ${unported_in[$bpath]:-0} + 1 )) ;;
      ported)   ported_in[$bpath]=$(( ${ported_in[$bpath]:-0} + 1 )) ;;
    esac
  done <"$behavior_prev"
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
    # Progress notes written into the ledger by hand (port fences, "no surface
    # yet") are not derivable from the pin, so a regen must carry them over.
    if [[ -z $reason && -n ${prev_module_reason[$rel]+x} ]]; then
      reason="${prev_module_reason[$rel]}"
    fi
    src_key="${rel#packages/effect/src/}"
    src_key="${src_key%.ts}"
    tests_field=""
    # The relative key wins when it resolves. `src/ai/Prompt.ts` and
    # `src/cli/Prompt.ts` share a basename and must stay apart, which is why
    # the directory is the primary route and never a tiebreaker.
    link_keys=("$src_key")
    if [[ -z ${tests_for[$src_key]+x} ]]; then
      # Fall back to the basename, but only to a file no other module links
      # relatively and only when that leaves exactly one candidate. An
      # ambiguous basename links nothing rather than guessing, and a test file
      # already spoken for keeps its owner.
      base="${src_key##*/}"
      if [[ -n ${tests_by_base[$base]+x} ]]; then
        unclaimed=()
        while IFS= read -r t; do
          [[ -z $t ]] && continue
          [[ -n ${claimed_by_relative[$t]+x} ]] && continue
          unclaimed+=("$t")
        done < <(printf '%s\n' "${tests_by_base[$base]}")
        if (( ${#unclaimed[@]} == 1 )); then
          k="${unclaimed[0]#packages/effect/test/}"
          link_keys=("${k%.test.ts}")
        fi
      fi
    fi
    for link_key in "${link_keys[@]}"; do
      [[ -z ${tests_for[$link_key]+x} ]] && continue
      while IFS= read -r test_rel; do
        [[ -z $test_rel ]] && continue
        if [[ -n $tests_field ]]; then
          tests_field+=", "
        fi
        tests_field+="\"$test_rel\""
      done < <(printf '%s\n' "${tests_for[$link_key]}" | sort)
    done
    # ADR-0002:100 + :565. `matched` is derived, never carried. A module
    # claims parity only when its linked test file has AT LEAST ONE PORTED row
    # and NO unported row. The ported count is load-bearing, not a nicety:
    # without it a module whose every row was excused as not-applicable would
    # read as matched parity with zero tests run, which is the exact parity
    # theater :565 names ("types without tests"). An experiment that excused
    # all 6940 open rows silently promoted 63 modules to matched that way,
    # JsonSchema.ts among them.
    if [[ $class == port || $class == unstable-port || $class == experimental-port ]]; then
      linked_rows=0
      linked_ported=0
      linked_unported=0
      # tests_field is '"path", "path"': strip quotes and the comma
      # separator, or every key misses and every module reads as testless.
      for t in ${tests_field//[\",]/ }; do
        [[ -z $t ]] && continue
        linked_rows=$(( linked_rows + ${rows_in[$t]:-0} ))
        linked_ported=$(( linked_ported + ${ported_in[$t]:-0} ))
        linked_unported=$(( linked_unported + ${unported_in[$t]:-0} ))
      done
      if (( linked_rows == 0 || linked_ported == 0 || linked_unported > 0 )); then
        status="unmatched"
      else
        status="matched"
      fi
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
# helpers that are not named .test.ts. Chained declarations (it.effect.each,
# it.effect.skipIf, it.live.prop) record their call form as kind. Status
# survives a regen when path, line, and title are unchanged.
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
# Prefer the quote character the line actually opens with: a template title can
# hold quoted fragments (`... ${x ? "a" : "b"} ...`) that must not win.
take_title() {
  local chunk="$1" lead
  lead="${chunk#"${chunk%%[![:space:]]*}"}"
  case "$lead" in
    \"*)
      if [[ $chunk =~ \"([^\"]*)\" ]]; then
        title="${BASH_REMATCH[1]}"
      fi
      ;;
    \'*)
      if [[ $chunk =~ \'([^\']*)\' ]]; then
        title="${BASH_REMATCH[1]}"
      fi
      ;;
    \`*)
      if [[ $chunk =~ \`([^\`]*)\` ]]; then
        title="${BASH_REMATCH[1]}"
      fi
      ;;
    *)
      if [[ $chunk =~ \"([^\"]*)\" ]]; then
        title="${BASH_REMATCH[1]}"
      elif [[ $chunk =~ \'([^\']*)\' ]]; then
        title="${BASH_REMATCH[1]}"
      elif [[ $chunk =~ \`([^\`]*)\` ]]; then
        title="${BASH_REMATCH[1]}"
      fi
      ;;
  esac
}
while IFS= read -r file; do
  rel="${file#"$checkout"/}"
  mapfile -t lines <"$file"
  for ((i = 0; i < ${#lines[@]}; i++)); do
    line="${lines[$i]}"
    [[ $line =~ ^[[:space:]]*(it(\.[A-Za-z]+)*|test)\( ]] || continue
    kind="${BASH_REMATCH[1]}"
    if [[ $kind == it.* ]]; then
      # Declarations chain modifiers: it.effect.each, it.effect.skipIf,
      # it.live.prop. The ledger kind is the call form (it.each), so a chained
      # row counts beside its plain siblings. Anything else (it.isFailure,
      # it.mock.calls) is a matcher, not a test declaration.
      chain="${kind#it.}"
      kind=""
      while :; do
        seg="${chain%%.*}"
        case "$seg" in
          effect | live | layer | each | skipIf | skip | prop) kind="it.$seg" ;;
          *) kind=""; break ;;
        esac
        [[ $chain == *.* ]] || break
        chain="${chain#*.}"
      done
      [[ -z $kind ]] && continue
    fi
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
    elif [[ $kind == it.skipIf ]]; then
      # A skipIf condition is arbitrary code and may hold quoted strings
      # (`it.effect.skipIf(version !== "2025-11-25")(`), so the title is what
      # follows the argument list, never the first quote on the line.
      if [[ $line =~ \)[[:space:]]*\([[:space:]]*\"([^\"]*)\" ]]; then
        title="${BASH_REMATCH[1]}"
      elif [[ $line =~ \)[[:space:]]*\([[:space:]]*\'([^\']*)\' ]]; then
        title="${BASH_REMATCH[1]}"
      elif [[ $line =~ \)[[:space:]]*\([[:space:]]*\`([^\`]*)\` ]]; then
        title="${BASH_REMATCH[1]}"
      elif [[ $line =~ \)[[:space:]]*\([[:space:]]*$ && $((i + 1)) -lt ${#lines[@]} ]]; then
        take_title "${lines[$((i + 1))]}"
      fi
      if [[ -z $title ]]; then
        for ((j = i + 1; j < i + 8 && j < ${#lines[@]}; j++)); do
          take_title "${lines[$j]}"
          [[ -n $title ]] && break
        done
      fi
      if [[ -z $title && $line =~ \(([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*, ]]; then
        title="${BASH_REMATCH[1]}"
      fi
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
