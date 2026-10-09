#!/bin/sh
# Product finish line. ADR-0002 witness list. Not the Effect ledger.
set -eu
cd "$(dirname "$0")/.."

run() {
  # shellcheck disable=SC2068
  cargo test "$@" -- --exact
}

run -p elm-effect --test l3_effect description_drop_runs_nothing
run -p elm-effect --test l2_cause five_outcomes_do_not_collapse
run -p elm-effect --test l5_scope release_on_three_exits
run -p elm-effect --test remote attempt_timeout_does_not_cover_the_queue
run -p elm-effect-test --test clock adjust_before_sleep_is_scheduled_does_not_complete_it
run -p elm-effect --test layer cycle_fails_construction
run -p elm-effect-elm --test loop_joint

echo "witness: product finish line passed"
