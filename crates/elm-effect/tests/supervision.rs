use elm_effect::{FiberId, Supervisor};

#[test]
fn supervisor_tracks_children_and_dumps_them() {
    let sup = Supervisor::new();
    sup.watch(FiberId(1), None);
    sup.watch(FiberId(2), Some(FiberId(1)));
    sup.watch(FiberId(2), Some(FiberId(1))); // re-watch is idempotent
    assert_eq!(sup.children().len(), 2);
    let dump = sup.dump();
    assert!(dump.contains('1'), "dump names every child; got:\n{dump}");
    assert!(dump.contains('2'), "dump names every child; got:\n{dump}");
    assert!(sup.unwatch(FiberId(1)));
    assert!(!sup.unwatch(FiberId(1)));
    assert_eq!(sup.children().len(), 1);
}

#[test]
fn escalation_names_known_and_unknown_stragglers() {
    let sup = Supervisor::new();
    sup.watch(FiberId(7), Some(FiberId(1)));
    let esc = sup.escalate(&[FiberId(7), FiberId(9)]);
    assert_eq!(esc.known.len(), 1);
    assert_eq!(esc.unknown, vec![FiberId(9)]);
    let msg = esc.message();
    assert!(
        msg.contains('7') && msg.contains('9'),
        "escalation names every straggler; got:\n{msg}"
    );
}
