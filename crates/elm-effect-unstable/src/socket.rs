//! Socket pair (ADR-0002 Phase 12 platform substitutes).
//!
//! [`socket_pair`] links two [`End`]s: what one writes, the other reads,
//! one buffered message at a time. Writers park while the peer buffer
//! holds at least `high_water_mark` bytes (text counts UTF-8 bytes).
//! Dropping an end closes it: the peer drains what remains, then reads
//! resolve [`None`]. [`End::reader_string`] maps pulls to UTF-8 text.
//!
//! One state lock guards queues, counts, and flags together with a
//! single [`Condvar`](std::sync::Condvar): check-and-wait is atomic, so
//! no wakeup is ever lost between drain and park.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// Which side of the pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// First end.
    A,
    /// Second end.
    B,
}

struct State {
    a_to_b: VecDeque<Vec<u8>>,
    b_to_a: VecDeque<Vec<u8>>,
    buffered_a_to_b: usize,
    buffered_b_to_a: usize,
    closed_a: bool,
    closed_b: bool,
}

struct Channel {
    state: Mutex<State>,
    high_water: usize,
    wake: Condvar,
}

impl Channel {
    fn peer_closed(&self, state: &State, side: Side) -> bool {
        match side {
            Side::A => state.closed_b,
            Side::B => state.closed_a,
        }
    }
}

/// One end of a socket pair. Clone shares the end.
#[derive(Clone)]
pub struct End {
    channel: Arc<Channel>,
    side: Side,
}

impl End {
    /// Write one message; parks while the peer buffer is at or above
    /// the high-water mark. A closed peer ends the write silently.
    pub fn write(&self, bytes: Vec<u8>) {
        let mut state = self.channel.state.lock().unwrap();
        loop {
            if Channel::peer_closed(&self.channel, &state, self.side) {
                return;
            }
            let buffered = match self.side {
                Side::A => state.buffered_a_to_b,
                Side::B => state.buffered_b_to_a,
            };
            if buffered < self.channel.high_water {
                break;
            }
            state = self.channel.wake.wait(state).unwrap();
        }
        // Re-check under the same lock: no lost wakeup possible.
        if Channel::peer_closed(&self.channel, &state, self.side) {
            return;
        }
        let len = bytes.len();
        match self.side {
            Side::A => {
                state.a_to_b.push_back(bytes);
                state.buffered_a_to_b += len;
            }
            Side::B => {
                state.b_to_a.push_back(bytes);
                state.buffered_b_to_a += len;
            }
        }
        self.channel.wake.notify_all();
    }

    /// Next buffered message, or [`None`] once the peer closed and the
    /// buffer drained.
    pub fn reader_bytes(&self) -> Option<Vec<u8>> {
        let mut state = self.channel.state.lock().unwrap();
        loop {
            let msg = match self.side {
                Side::A => state.b_to_a.pop_front(),
                Side::B => state.a_to_b.pop_front(),
            };
            if let Some(msg) = msg {
                match self.side {
                    Side::A => state.buffered_b_to_a -= msg.len(),
                    Side::B => state.buffered_a_to_b -= msg.len(),
                }
                self.channel.wake.notify_all();
                return Some(msg);
            }
            if Channel::peer_closed(&self.channel, &state, self.side) {
                return None;
            }
            state = self.channel.wake.wait(state).unwrap();
        }
    }

    /// Next buffered message, waiting at most `timeout`.
    pub fn reader_bytes_timeout(&self, timeout: std::time::Duration) -> Option<Vec<u8>> {
        let mut state = self.channel.state.lock().unwrap();
        loop {
            let msg = match self.side {
                Side::A => state.b_to_a.pop_front(),
                Side::B => state.a_to_b.pop_front(),
            };
            if let Some(msg) = msg {
                match self.side {
                    Side::A => state.buffered_b_to_a -= msg.len(),
                    Side::B => state.buffered_a_to_b -= msg.len(),
                }
                self.channel.wake.notify_all();
                return Some(msg);
            }
            if Channel::peer_closed(&self.channel, &state, self.side) {
                return None;
            }
            let (guard, result) = self.channel.wake.wait_timeout(state, timeout).unwrap();
            state = guard;
            if result.timed_out() {
                return None;
            }
        }
    }

    /// Next buffered message as UTF-8 text.
    pub fn reader_string(&self) -> Option<Result<String, String>> {
        self.reader_bytes().map(|bytes| {
            String::from_utf8(bytes).map_err(|_| "invalid UTF-8".to_string())
        })
    }
}

impl Drop for End {
    fn drop(&mut self) {
        let mut state = self.channel.state.lock().unwrap();
        match self.side {
            Side::A => state.closed_a = true,
            Side::B => state.closed_b = true,
        }
        self.channel.wake.notify_all();
    }
}

/// Linked pair sharing one high-water mark.
pub fn socket_pair(high_water: usize) -> (End, End) {
    let channel = Arc::new(Channel {
        state: Mutex::new(State {
            a_to_b: VecDeque::new(),
            b_to_a: VecDeque::new(),
            buffered_a_to_b: 0,
            buffered_b_to_a: 0,
            closed_a: false,
            closed_b: false,
        }),
        high_water: high_water.max(1),
        wake: Condvar::new(),
    });
    (
        End { channel: channel.clone(), side: Side::A },
        End { channel, side: Side::B },
    )
}
