//! A cron firing is one durable start (ADR-0005).

use elm_effect_durable::{
    CronFire, CronPolicy, CronSchedule, DurableEngine, OurEngine, StartOutcome, fire_key,
    plan_cron_tick,
};

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn ms(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> u64 {
    let days = days_from_civil(year, month, day);
    ((days * 86_400 + hour * 3_600 + minute * 60) * 1_000) as u64
}

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
fn step_over_wildcard_leaves_dom_unrestricted() {
    // `*/1` covers every day but stays a wildcard: only Monday may match.
    let cron = CronSchedule::parse("0 0 */1 * MON").expect("valid cron");
    // Tuesday 2024-09-10 -> next Monday 2024-09-16. An OR rule would
    // have fired on day-of-month alone (2024-09-11).
    assert_eq!(
        cron.next_after_ms(ms(2024, 9, 10, 0, 0)),
        Some(ms(2024, 9, 16, 0, 0))
    );
}

#[test]
fn explicit_days_stay_restricted_or_rule() {
    let cron = CronSchedule::parse("0 0 13 * FRI").expect("valid cron");
    // Sunday 2024-10-13 is the 13th: day-of-month alone matches.
    assert_eq!(
        cron.next_after_ms(ms(2024, 10, 12, 0, 0)),
        Some(ms(2024, 10, 13, 0, 0))
    );
}

#[test]
fn epoch_boundary_is_clean() {
    let cron = CronSchedule::parse("* * * * *").expect("valid cron");
    // No tick is strictly after 0 and at-or-before 0; no underflow, no skip.
    assert_eq!(cron.latest_at_or_before_ms(0, 0), None);
    assert_eq!(cron.latest_at_or_before_ms(0, 60_000), Some(60_000));
    assert_eq!(cron.next_after_ms(0), Some(60_000));
}

#[test]
fn plan_latest_tick_never_catches_up_history_for_new_schedules() {
    let cron = CronSchedule::parse("0 0 * * *").expect("valid cron");
    let noon = ms(2024, 9, 6, 12, 0);
    // Brand-new schedule at noon: midnight is hours old and only the last
    // minute is considered, so there is no false historical catch-up.
    assert_eq!(cron.plan_latest_tick("nightly", None, noon), None);
    // But a matching current minute still fires.
    let midnight = ms(2024, 9, 6, 0, 0);
    let just_after = midnight + 30_000;
    assert_eq!(
        cron.plan_latest_tick("nightly", None, just_after),
        Some((fire_key("nightly", midnight), midnight))
    );
    // A known last-firing resumes the normal one-tick catch-up.
    let last = ms(2024, 9, 5, 0, 0);
    assert_eq!(
        cron.plan_latest_tick("nightly", Some(last), noon),
        Some((fire_key("nightly", midnight), midnight))
    );
    // fields() exposes the parsed bitmasks for the host tick loop.
    let (minutes, hours, _, _, _) = cron.fields();
    assert_eq!(minutes, 1u64 << 0);
    assert_eq!(hours, 1u64 << 0);
}

#[test]
fn plan_skip_tick_honors_grace() {
    let cron = CronSchedule::parse("* * * * *").expect("valid cron");
    // 30s into the minute: fresh under a 60s grace, stale under 10s.
    let now_ms = ms(2024, 9, 6, 12, 0) + 30_000;
    let tick = ms(2024, 9, 6, 12, 0);
    assert_eq!(
        cron.plan_skip_tick("beat", 60_000, None, now_ms),
        Some((fire_key("beat", tick), tick))
    );
    assert_eq!(cron.plan_skip_tick("beat", 10_000, None, now_ms), None);
    // The Some(last) path advances one tick past the last firing.
    let tick2 = tick + 60_000;
    assert_eq!(
        cron.plan_skip_tick("beat", 60_000, Some(tick), tick2 + 30_000),
        Some((fire_key("beat", tick2), tick2))
    );
    // A future tick never fires early, however generous the grace.
    let daily = CronSchedule::parse("0 0 * * *").expect("valid cron");
    assert_eq!(
        daily.plan_skip_tick("daily", 3_600_000, Some(now_ms), now_ms),
        None
    );
}

#[test]
fn deterministic_keys() {
    assert_eq!(
        CronSchedule::child_key("run-1", "fetch", 2),
        "run-1:fetch:2"
    );
    assert_eq!(CronSchedule::step_key("run-1", "fetch"), "run-1:fetch");
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

#[test]
fn cron_policy_latest_starts_once_through_engine() {
    let schedule = CronSchedule::parse("* * * * *").expect("valid cron");
    let tick = ms(2024, 9, 6, 12, 0);
    let now_ms = tick + 30_000;
    // Free function and method planner agree on the same firing.
    let fire = plan_cron_tick("beat", &schedule, CronPolicy::Latest, None, now_ms)
        .expect("latest fires a fresh tick");
    assert_eq!(fire.tick_ms, tick);
    assert_eq!(fire.key, fire_key("beat", tick));
    assert_eq!(
        schedule.plan_tick("beat", CronPolicy::Latest, None, now_ms),
        Some(fire.clone())
    );
    // A CronFire is data: it round-trips through serde.
    let back: CronFire =
        serde_json::from_str(&serde_json::to_string(&fire).expect("json")).expect("back");
    assert_eq!(back, fire);

    let mut engine = OurEngine::new();
    assert!(
        matches!(
            engine.start(&fire.key, now_ms),
            Ok(StartOutcome::Started { .. })
        ),
        "first firing dispatches"
    );
    assert!(
        matches!(
            engine.start(&fire.key, now_ms),
            Ok(StartOutcome::Already { .. })
        ),
        "replay re-attaches instead of dispatching twice"
    );
}

#[test]
fn cron_policy_skip_grace_starts_once_through_engine() {
    let schedule = CronSchedule::parse("* * * * *").expect("valid cron");
    let tick = ms(2024, 9, 6, 12, 0);
    let now_ms = tick + 30_000;
    let policy = CronPolicy::Skip { grace_ms: 60_000 };
    let fire = plan_cron_tick("beat", &schedule, policy, None, now_ms).expect("fresh tick fires");
    assert_eq!(
        schedule.plan_tick("beat", policy, None, now_ms),
        Some(fire.clone())
    );
    // A stale tick never fires, however the engine would have taken it.
    assert!(
        plan_cron_tick(
            "beat",
            &schedule,
            CronPolicy::Skip { grace_ms: 10_000 },
            None,
            now_ms
        )
        .is_none()
    );

    let mut engine = OurEngine::new();
    assert!(
        matches!(
            engine.start(&fire.key, now_ms),
            Ok(StartOutcome::Started { .. })
        ),
        "first firing dispatches"
    );
    assert!(
        matches!(
            engine.start(&fire.key, now_ms),
            Ok(StartOutcome::Already { .. })
        ),
        "replay re-attaches instead of dispatching twice"
    );
}
