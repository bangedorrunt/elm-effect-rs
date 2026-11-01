//! A cron firing is one durable start (ADR-0005).

use elm_effect_durable::{CronSchedule, DurableEngine, OurEngine, StartOutcome, fire_key};

#[test]
fn one_cron_tick_starts_once() {
    let schedule = CronSchedule::parse("*/15 * * * *").expect("valid cron");
    let now_ms = 1_700_000_000_000;
    let tick_ms = schedule.next_after_ms(now_ms).expect("next tick");
    let key = fire_key("digest", tick_ms);

    let mut engine = OurEngine::new();
    let first = engine.start(&key, now_ms).expect("first firing");
    assert!(matches!(first, StartOutcome::Started { .. }));
    let second = engine.start(&key, now_ms).expect("replay");
    assert!(
        matches!(second, StartOutcome::Already { .. }),
        "the same tick must re-attach, not dispatch twice: {second:?}"
    );

    let due = schedule
        .latest_at_or_before_ms(tick_ms - 1, tick_ms)
        .expect("the tick is due");
    assert_eq!(due, tick_ms);
    assert_eq!(schedule.latest_at_or_before_ms(tick_ms, tick_ms + 1), None);
    assert!(schedule.next_after_ms(tick_ms).is_some());
}

#[test]
fn one_catch_up_tick_starts_once() {
    let schedule = CronSchedule::parse("0 8 * * *").expect("valid cron");
    let last_ms = 1_700_000_000_000;
    let now_ms = last_ms + 3 * 86_400_000;
    let tick_ms = schedule
        .latest_at_or_before_ms(last_ms, now_ms)
        .expect("one missed tick");

    let mut engine = OurEngine::new();
    let key = fire_key("daily", tick_ms);
    assert!(matches!(
        engine.start(&key, now_ms),
        Ok(StartOutcome::Started { .. })
    ));
    assert!(matches!(
        engine.start(&key, now_ms),
        Ok(StartOutcome::Already { .. })
    ));
}
