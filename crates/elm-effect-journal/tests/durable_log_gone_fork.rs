//! Gone/close fence: lifecycle, retention vs delete, snapshots.
//! Loads `stream.rs` by path until lib.rs gains the mod line
//! (CopperCoast-held for .46).

#[path = "../src/stream.rs"]
mod stream;

use stream::{Gone, Read, Status, Stream};

#[test]
fn up_to_date_is_not_closed() {
    let s = Stream::new(8);
    assert_eq!(s.status(), Status::UpToDate);
    assert_ne!(s.status(), Status::Closed);
}

#[test]
fn append_after_close_fails() {
    let mut s = Stream::new(8);
    s.append(vec![1]).unwrap();
    s.close();
    s.close();
    assert_eq!(s.status(), Status::Closed);
    assert!(s.append(vec![2]).is_err());
}

#[test]
fn closed_stream_stays_readable() {
    let mut s = Stream::new(8);
    s.append(vec![1]).unwrap();
    s.append(vec![2]).unwrap();
    s.close();
    assert!(matches!(s.read(0), Read::Entry { .. }));
    assert!(matches!(s.read(1), Read::Entry { .. }));
}

#[test]
fn retention_gone_is_not_a_skip() {
    let mut s = Stream::new(2);
    s.append(vec![1]).unwrap();
    s.append(vec![2]).unwrap();
    s.append(vec![3]).unwrap();
    match s.read(0).unwrap_err_gone() {
        Gone::Retention { first_replayable } => assert_eq!(first_replayable, 1),
        Gone::SoftDeleted => panic!("retention is not a delete"),
    }
}

#[test]
fn soft_delete_gone_is_not_retention() {
    let mut s = Stream::new(8);
    s.append(vec![1]).unwrap();
    s.soft_delete();
    match s.read(0).unwrap_err_gone() {
        Gone::SoftDeleted => {}
        Gone::Retention { .. } => panic!("delete is not retention"),
    }
    assert!(s.append(vec![2]).is_err());
}

#[test]
fn snapshot_is_model_plus_offset() {
    let mut s = Stream::new(8);
    s.append(vec![1]).unwrap();
    s.append(vec![2]).unwrap();
    s.save_snapshot(vec![9, 9], 1);
    let snap = s.snapshot().expect("snapshot stored");
    assert_eq!(snap.model, vec![9, 9]);
    assert_eq!(snap.offset, 1);
}

trait UnwrapGone {
    fn unwrap_err_gone(self) -> Gone;
}

impl UnwrapGone for Read {
    fn unwrap_err_gone(self) -> Gone {
        match self {
            Read::Gone(g) => g,
            Read::Entry { .. } => panic!("expected gone"),
        }
    }
}
