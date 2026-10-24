//! Schedules as values; retry repeats typed failures only.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use elm_effect::{
    Decision, Defect, Duration, Effect, Exit, Random, Runtime, Schedule, UpTo, repeat, retry,
};

#[test]
fn recurs_counts_retries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let make = {
        let calls = Arc::clone(&calls);
        move || {
            let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 3 {
                Effect::<u32, String, ()>::fail("flaky".to_string())
            } else {
                Effect::succeed(n as u32)
            }
        }
    };
    let exit = Runtime::run(retry(make, Schedule::recurs(2)));
    assert!(matches!(exit, Exit::Success(3)));
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    // max stops when every member stops: recurs(1) ends it after 1 retry.
    let calls = Arc::new(AtomicUsize::new(0));
    let policy = Schedule::recurs(1).max(Schedule::recurs(3));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::fail("x".to_string())
            }
        },
        policy,
    ));
    assert!(matches!(exit, Exit::Failure(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // min continues while any member continues: recurs(3) carries to 3 retries.
    let calls = Arc::new(AtomicUsize::new(0));
    let policy = Schedule::recurs(1).min(Schedule::recurs(3));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::fail("x".to_string())
            }
        },
        policy,
    ));
    assert!(matches!(exit, Exit::Failure(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[test]
fn max_min_choose_delays() {
    let long = Schedule::spaced(Duration::from_millis(10))
        .max(Schedule::spaced(Duration::from_millis(30)));
    let (_, delay) = long.decide(0, &"e".to_string()).unwrap();
    assert_eq!(delay.as_millis(), 30);

    let short = Schedule::spaced(Duration::from_millis(10))
        .min(Schedule::spaced(Duration::from_millis(30)));
    let (_, delay) = short.decide(0, &"e".to_string()).unwrap();
    assert_eq!(delay.as_millis(), 10);
}

#[test]
fn jitter_is_deterministic_per_seed() {
    let policy = Schedule::<String, Duration>::exponential(Duration::from_millis(20))
        .jitter(Random::with_seed(7));
    // Each schedule counts its own steps, so the wrapped `exponential` is on
    // its first attempt (20ms) whatever index the caller passes.
    let (_, d1) = policy.decide(0, &"e".to_string()).unwrap();
    let again = Schedule::<String, Duration>::exponential(Duration::from_millis(20))
        .jitter(Random::with_seed(7));
    let (_, d2) = again.decide(0, &"e".to_string()).unwrap();
    assert_eq!(d1.as_millis(), d2.as_millis());
    // Base 20ms scaled by the pin's jitter band, [0.8, 1.2).
    assert!((16..24).contains(&d1.as_millis()));
}

/// The pin's `runDelays`: step the schedule over a virtual clock that advances
/// by each decided delay, recording the delay, with a trailing zero where the
/// schedule completes. `toStepWithMetadata` sleeps the delay at the pin, so the
/// clock advance is what makes a time-dependent policy observably the same.
fn run_delays_from<In, A>(schedule: &Schedule<In, A>, inputs: &[In], start: f64) -> Vec<u64>
where
    In: Send + Sync + 'static,
    A: Send + 'static,
{
    let mut stepper = schedule.to_step();
    let mut now = start;
    let mut out = Vec::new();
    for input in inputs {
        match stepper.step_with_metadata(now, input) {
            Some(metadata) => {
                out.push(metadata.duration.as_millis());
                now += metadata.duration.as_millis() as f64;
            }
            None => {
                out.push(0);
                break;
            }
        }
    }
    out
}

fn run_delays<In, A>(schedule: &Schedule<In, A>, inputs: &[In]) -> Vec<u64>
where
    In: Send + Sync + 'static,
    A: Send + 'static,
{
    run_delays_from(schedule, inputs, 0.0)
}

#[test]
fn schedule_constructors_and_combining_match_the_pin() {
    // Schedule.test.ts:500, :510 — spaced and fixed hold a constant delay.
    assert_eq!(
        run_delays(
            &Schedule::<(), u64>::spaced(Duration::from_secs(1)),
            &[(); 5]
        ),
        vec![1000; 5]
    );
    assert_eq!(
        run_delays(
            &Schedule::<(), u64>::fixed(Duration::from_secs(1)),
            &[(); 5]
        ),
        vec![1000; 5]
    );

    // :21 — max waits the slowest member.
    let slowest = Schedule::<(), u64>::fixed(Duration::from_secs(5))
        .max(Schedule::exponential(Duration::from_secs(5)))
        .max(Schedule::spaced(Duration::from_secs(10)));
    assert_eq!(run_delays(&slowest, &[(); 3]), vec![10_000, 10_000, 20_000]);

    // :51 — min waits the fastest member.
    let fastest = Schedule::<(), u64>::fixed(Duration::from_secs(5))
        .min(Schedule::exponential(Duration::from_secs(5)))
        .min(Schedule::spaced(Duration::from_secs(10)));
    assert_eq!(run_delays(&fastest, &[(); 3]), vec![5_000, 5_000, 5_000]);

    // :476 — jittered stays inside 80%-120% of the base delay.
    let jittered =
        Schedule::<(), u64>::spaced(Duration::from_secs(1)).jitter(Random::with_seed(11));
    for attempt in 0..20 {
        let (_, delay) = jittered.decide(attempt, &()).unwrap();
        assert!(
            (800..=1200).contains(&delay.as_millis()),
            "attempt {attempt} left the band: {}ms",
            delay.as_millis()
        );
    }
}

#[test]
fn schedule_once_and_up_to_match_the_pin() {
    // :379 — once continues on the first step, then completes.
    let once = Schedule::<&'static str, ()>::once();
    assert_eq!(
        once.decide(0, &"first").map(|(_, d)| d.as_millis()),
        Some(0)
    );
    assert!(once.decide(1, &"second").is_none());

    // :405 — upTo({ times: 2 }) lets two through and echoes the input.
    let capped = Schedule::<&'static str, &'static str>::identity().up_to_times(2);
    assert_eq!(capped.decide(0, &"a").map(|(out, _)| out), Some("a"));
    assert_eq!(capped.decide(1, &"b").map(|(out, _)| out), Some("b"));
    assert!(capped.decide(2, &"c").is_none());

    // :393 — repeat(once) executes the effect exactly twice.
    let runs = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(repeat(
        {
            let runs = Arc::clone(&runs);
            move || {
                runs.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::succeed(7)
            }
        },
        Schedule::<u32, ()>::once(),
    ));
    assert!(matches!(exit, Exit::Success(7)));
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[test]
fn retry_never_retries_die_or_interrupt() {
    // Defects pass through on first sight.
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::die(Defect::new("kaput"))
            }
        },
        Schedule::recurs(5),
    ));
    match exit {
        Exit::Failure(c) => assert_eq!(c.defects().len(), 1),
        Exit::Success(_) => panic!("expected failure"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Interrupts pass through on first sight.
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::interrupt()
            }
        },
        Schedule::recurs(5),
    ));
    match exit {
        Exit::Failure(c) => assert!(c.is_interrupted()),
        Exit::Success(_) => panic!("expected failure"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Typed failures do retry.
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::fail("nope".to_string())
            }
        },
        Schedule::recurs(2),
    ));
    assert!(matches!(exit, Exit::Failure(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn repeat_reruns_success() {
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(repeat(
        {
            let calls = Arc::clone(&calls);
            move || {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                Effect::<u32, String, ()>::succeed(n as u32)
            }
        },
        Schedule::recurs(2),
    ));
    assert!(matches!(exit, Exit::Success(3)));
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    // A failure stops a repeat immediately.
    let exit = Runtime::run(repeat(
        || Effect::<u32, String, ()>::fail("stop".to_string()),
        Schedule::recurs(5),
    ));
    assert!(matches!(exit, Exit::Failure(_)));
}

/// A metadata snapshot the way the pin's rows spell it: input, output,
/// duration, attempt, and the four timings.
type Snapshot = (String, String, u64, u64, f64, f64, f64, f64);

fn snapshot<In: ToString, Out: ToString>(meta: &elm_effect::Metadata<'_, In, Out>) -> Snapshot {
    (
        meta.input.to_string(),
        meta.output.to_string(),
        meta.duration.as_millis(),
        meta.attempt,
        meta.start,
        meta.now,
        meta.elapsed,
        meta.elapsed_since_previous,
    )
}

#[test]
fn during_recurs_inside_the_window() {
    // Schedule.test.ts:8 — the first step has not outlived 1 second.
    let window = Schedule::<(), Duration>::during(Duration::from_secs(1));
    let mut stepper = window.to_step();
    assert!(matches!(stepper.step(0.0, &()), Decision::Continue { .. }));
    // Past the limit the same step completes, carrying the elapsed time.
    match stepper.step(1_001.0, &()) {
        Decision::Done { output } => {
            assert_eq!(output.map(|d| d.as_millis()), Some(1_001));
        }
        Decision::Continue { .. } => panic!("during kept going past its limit"),
    }
}

#[test]
fn duration_stops_the_max_and_runs_out_on_min() {
    // :37 — max stops when any member completes; the trailing zero is the
    // completion runDelays records.
    let halted = Schedule::<(), Duration>::duration(Duration::from_secs(1))
        .max(Schedule::spaced(Duration::from_secs(5)));
    assert_eq!(run_delays(&halted, &[(); 3]), vec![5_000, 0]);

    // :67 — min continues after one member completes, waiting the survivor.
    let surviving = Schedule::<(), Duration>::duration(Duration::from_secs(1))
        .min(Schedule::spaced(Duration::from_secs(5)));
    assert_eq!(run_delays(&surviving, &[(); 3]), vec![1_000, 5_000, 5_000]);
}

#[test]
fn concat_sequences_left_then_right() {
    // :84 — two left steps, then the right side supplies the third.
    let left = Schedule::<&'static str, &'static str>::identity()
        .up_to_times(2)
        .map(|input| format!("left:{input}"));
    let right =
        Schedule::<&'static str, &'static str>::identity().map(|input| format!("right:{input}"));
    let schedule = left.concat(right);
    let mut stepper = schedule.to_step();
    let outputs: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|input| match stepper.step(0.0, input) {
            Decision::Continue { output, .. } => output,
            Decision::Done { .. } => panic!("concat stopped before the right side"),
        })
        .collect();
    assert_eq!(outputs, vec!["left:a", "left:b", "right:c"]);
}

#[test]
fn concat_result_hands_over_and_tags_each_side() {
    // :264 — self's outputs are Err, other's are Ok.
    let left = Schedule::<&'static str, &'static str>::identity().up_to_times(2);
    let right = Schedule::<&'static str, &'static str>::identity();
    let schedule = left.concat_result(right);
    let mut stepper = schedule.to_step();
    let tagged: Vec<std::result::Result<&'static str, &'static str>> =
        ["left-1", "left-2", "right-1"]
            .iter()
            .map(|input| match stepper.step(0.0, input) {
                Decision::Continue { output, .. } => output,
                Decision::Done { .. } => panic!("concatResult stopped early"),
            })
            .collect();
    assert_eq!(tagged, vec![Err("left-1"), Err("left-2"), Ok("right-1")]);
}

#[test]
fn concat_result_collects_delays_across_the_handover() {
    // :228 — three 500ms steps from the left, then the right side's own
    // one-second windows. The right side counts its own attempts, so its
    // first step is attempt 1 even though the whole schedule has stepped
    // four times.
    let left = Schedule::<(), u64>::fixed(Duration::from_millis(500))
        .while_metadata(|meta| meta.attempt <= 3);
    let right = Schedule::<(), u64>::fixed(Duration::from_secs(1));
    let schedule = left.concat_result(right);
    assert_eq!(
        run_delays(&schedule, &[(); 6]),
        vec![500, 500, 500, 1_000, 1_000, 1_000]
    );

    // :247 — the right side's completion is the schedule's completion.
    let left = Schedule::<(), u64>::fixed(Duration::from_millis(500))
        .while_metadata(|meta| meta.attempt <= 2);
    let right = Schedule::<(), Duration>::duration(Duration::from_secs(1));
    let schedule = left.concat_result(right);
    assert_eq!(run_delays(&schedule, &[(); 5]), vec![500, 500, 1_000, 0]);
}

#[test]
fn tap_modify_and_add_delay_see_full_metadata() {
    // :106 — tap observes attempt, start, now, elapsed, and the previous gap.
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let schedule = Schedule::<&'static str, u64>::spaced(Duration::from_millis(250))
        .tap(move |meta| sink.lock().unwrap().push(snapshot(meta)));
    let mut stepper = schedule.to_step();
    let first = stepper.step(1_000.0, &"a");
    let second = stepper.step(1_250.0, &"b");
    assert!(matches!(
        first,
        Decision::Continue { duration, .. } if duration.as_millis() == 250
    ));
    assert!(matches!(
        second,
        Decision::Continue { duration, .. } if duration.as_millis() == 250
    ));
    let observed = observed.lock().unwrap().clone();
    assert_eq!(
        observed,
        vec![
            (
                "a".to_string(),
                "0".to_string(),
                250,
                1,
                1_000.0,
                1_000.0,
                0.0,
                0.0
            ),
            (
                "b".to_string(),
                "1".to_string(),
                250,
                2,
                1_000.0,
                1_250.0,
                250.0,
                250.0
            ),
        ]
    );

    // :146 — modifyDelay replaces the delay with what the metadata says.
    let modified =
        Schedule::<&'static str, u64>::spaced(Duration::from_millis(250)).modify_delay(|meta| {
            Duration::from_millis(meta.duration.as_millis() + meta.elapsed_since_previous as u64)
        });
    let mut stepper = modified.to_step();
    let first = stepper.step(1_000.0, &"a");
    let second = stepper.step(1_250.0, &"b");
    assert!(matches!(
        first,
        Decision::Continue { duration, .. } if duration.as_millis() == 250
    ));
    assert!(matches!(
        second,
        Decision::Continue { duration, .. } if duration.as_millis() == 500
    ));

    // :187 — addDelay sums the metadata delay onto the step's own.
    let added = Schedule::<&'static str, u64>::spaced(Duration::from_millis(250))
        .add_delay(|meta| Duration::from_millis(meta.elapsed_since_previous as u64));
    let mut stepper = added.to_step();
    let first = stepper.step(1_000.0, &"a");
    let second = stepper.step(1_250.0, &"b");
    assert!(matches!(
        first,
        Decision::Continue { duration, .. } if duration.as_millis() == 250
    ));
    assert!(matches!(
        second,
        Decision::Continue { duration, .. } if duration.as_millis() == 500
    ));
}

#[test]
fn cron_matches_the_pin() {
    // 2024-01-01T00:00:00Z, and 35 seconds past it.
    const MIDNIGHT: f64 = 1_704_067_200_000.0;
    const MIDNIGHT_35: f64 = 1_704_067_235_000.0;

    // :283 — every second minute, from :35 past midnight.
    let every_two = Schedule::<(), Duration>::cron("*/2 * * * *", None).expect("parse");
    assert_eq!(
        run_delays_from(&every_two, &[(); 4], MIDNIGHT_35),
        vec![85_000, 120_000, 120_000, 120_000]
    );

    // :304 — six-field form: every third second.
    let every_three = Schedule::<(), Duration>::cron("*/3 * * * * *", None).expect("parse");
    assert_eq!(
        run_delays_from(&every_three, &[(); 10], MIDNIGHT),
        vec![3_000; 10]
    );

    // :331 — 04:30 on the 5th, 15th, and every Wednesday.
    let wednesdays = Schedule::<(), Duration>::cron("30 4 5,15 * WED", None).expect("parse");
    assert_eq!(
        run_delays_from(&wednesdays, &[(); 6], MIDNIGHT),
        vec![
            189_000_000, // Wed Jan 3, 04:30
            172_800_000, // Fri Jan 5, 04:30 (day of month)
            432_000_000, // Wed Jan 10
            432_000_000, // Mon Jan 15 (day of month)
            172_800_000, // Wed Jan 17
            604_800_000, // Wed Jan 24
        ]
    );

    // :354 — a clock adjusted to infinity completes instead of failing.
    let zoned = Schedule::<(), Duration>::cron("0 0 4 8-14 * *", Some("UTC")).expect("parse");
    let mut stepper = zoned.to_step();
    assert!(matches!(
        stepper.step(f64::INFINITY, &()),
        Decision::Done { .. }
    ));
}

#[test]
fn duration_recurs_once_after_its_delay() {
    // :369 — one delayed recurrence, then completion.
    let once = Schedule::<(), Duration>::duration(Duration::from_secs(1));
    assert_eq!(run_delays(&once, &[(); 5]), vec![1_000, 0]);
}

#[test]
fn up_to_bounds_stop_with_the_refused_output() {
    // :425 — a duration bound refuses the step that crosses it, and the done
    // value is that step's own output.
    let by_duration = Schedule::<&'static str, &'static str>::identity().up_to(UpTo {
        times: None,
        duration: Some(Duration::from_millis(1_000)),
    });
    let mut stepper = by_duration.to_step();
    assert!(matches!(
        stepper.step(0.0, &"a"),
        Decision::Continue { output: "a", .. }
    ));
    assert!(matches!(
        stepper.step(1_000.0, &"b"),
        Decision::Continue { output: "b", .. }
    ));
    match stepper.step(1_001.0, &"c") {
        Decision::Done { output } => assert_eq!(output, Some("c")),
        Decision::Continue { .. } => panic!("the duration bound did not stop the step"),
    }

    // :445 — whichever option runs out first stops the schedule.
    let first_bound = Schedule::<&'static str, &'static str>::identity().up_to(UpTo {
        times: Some(1),
        duration: Some(Duration::from_secs(3_600)),
    });
    let mut stepper = first_bound.to_step();
    assert!(matches!(
        stepper.step(0.0, &"a"),
        Decision::Continue { output: "a", .. }
    ));
    match stepper.step(0.0, &"b") {
        Decision::Done { output } => assert_eq!(output, Some("b")),
        Decision::Continue { .. } => panic!("the times bound did not stop the step"),
    }

    // :460 — an options record with nothing set leaves the schedule alone.
    let unchanged = Schedule::<&'static str, &'static str>::identity().up_to(UpTo::default());
    let mut stepper = unchanged.to_step();
    for input in ["a", "b"] {
        assert!(matches!(
            stepper.step(0.0, &input),
            Decision::Continue { .. }
        ));
    }
}

#[test]
fn jitter_keeps_the_completion_output_alone() {
    // :487 — jitter scales the one recurrence; the completion stays zero.
    let schedule =
        Schedule::<(), Duration>::duration(Duration::from_secs(1)).jitter(Random::with_seed(3));
    let delays = run_delays(&schedule, &[(); 5]);
    assert_eq!(delays.len(), 2);
    assert!(
        (800..=1_200).contains(&delays[0]),
        "jittered delay left the band: {}ms",
        delays[0]
    );
    assert_eq!(delays[1], 0);
}

/// `fixed` under a repeating action: the clock advances by the action's own
/// duration plus the decided delay, which is what `Effect.schedule` does.
fn fixed_delays_with_action(action_ms: u64) -> Vec<u64> {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let schedule = Schedule::<(), u64>::fixed(Duration::from_secs(1))
        .while_metadata(|meta| meta.attempt <= 5)
        .tap(move |meta| sink.lock().unwrap().push(meta.duration.as_millis()));
    let mut stepper = schedule.to_step();
    let mut now = action_ms as f64;
    loop {
        match stepper.step_with_metadata(now, &()) {
            Some(metadata) => now += metadata.duration.as_millis() as f64 + action_ms as f64,
            None => break,
        }
    }
    let mut delays = seen.lock().unwrap().clone();
    // The pin's driver pushes a zero once the schedule completes.
    delays.push(0);
    delays
}

#[test]
fn fixed_trails_the_window_boundary() {
    // :518 — a 500ms action finishes inside its window, so each step waits the
    // remaining half second.
    assert_eq!(
        fixed_delays_with_action(500),
        vec![1_000, 500, 500, 500, 500, 0]
    );

    // :545 — a 1.5s action always starts behind its window, so it never waits.
    assert_eq!(fixed_delays_with_action(1_500), vec![1_000, 0, 0, 0, 0, 0]);
}

/// The same driver for `windowed`, whose step waits the remainder of its
/// window instead of skipping ahead when the action is slow. `steps` is the
/// number of inputs the driver offers, so a run that never completes records
/// no trailing zero.
fn windowed_delays(action_ms: u64, steps: usize) -> Vec<u64> {
    let schedule = Schedule::<(), u64>::windowed(Duration::from_secs(1))
        .while_metadata(|meta| meta.attempt <= 5);
    let mut stepper = schedule.to_step();
    let mut now = action_ms as f64;
    let mut delays = Vec::new();
    for _ in 0..steps {
        match stepper.step_with_metadata(now, &()) {
            Some(metadata) => {
                delays.push(metadata.duration.as_millis());
                now += metadata.duration.as_millis() as f64 + action_ms as f64;
            }
            None => {
                delays.push(0);
                break;
            }
        }
    }
    delays
}

#[test]
fn windowed_trails_the_boundary() {
    // :574 — with no action time, every window is a full second.
    assert_eq!(windowed_delays(0, 5), vec![1_000; 5]);

    // :582 — a 1.5s action straddles each window, so every later wait is the
    // half second left in the window it finished in.
    assert_eq!(
        windowed_delays(1_500, 6),
        vec![1_000, 500, 500, 500, 500, 0]
    );
}
