//! Bounded critical completion delivery. Full output never parks the SQL runtime.
use super::{ACCESS_READ_BATCH, MAX_JOBS, Reply};
use std::{
    collections::VecDeque,
    sync::mpsc::{SyncSender, TrySendError},
};

pub(super) struct CriticalReplies {
    pending: VecDeque<Reply>,
    receiver_closed: bool,
}
impl CriticalReplies {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::with_capacity(MAX_JOBS),
            receiver_closed: false,
        }
    }
    pub fn len(&self) -> usize {
        self.pending.len()
    }
    pub fn receiver_closed(&self) -> bool {
        self.receiver_closed
    }
    /// One Ready job may collect this entire parallel batch. Reserve space
    /// before receiving any job; only this worker thread can fill the budget.
    pub fn can_take_job(&self) -> bool {
        self.receiver_closed || self.pending.len() + ACCESS_READ_BATCH <= MAX_JOBS
    }
    pub fn retain(&mut self, reply: Reply) {
        if self.receiver_closed {
            return; // abandoned recipient; SQL/files remain the durable authority
        }
        debug_assert!(matches!(&reply, Reply::Ready(_) | Reply::Flushed(..)));
        assert!(
            self.pending.len() < MAX_JOBS,
            "critical reply budget must precede intake"
        );
        self.pending.push_back(reply);
    }
    pub fn poll(&mut self, sender: &SyncSender<Reply>) {
        while let Some(reply) = self.pending.pop_front() {
            match sender.try_send(reply) {
                Ok(()) => {}
                Err(TrySendError::Full(reply)) => {
                    self.pending.push_front(reply);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.receiver_closed = true;
                    self.pending.clear();
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::career_backend::consent::{Completion, Operation, Work};
    use omoba_career_store::career_store::ready::Action;
    use std::sync::mpsc;

    fn work() -> Work {
        Work {
            check_id: "b".repeat(32),
            sequence: 7,
            operation: Operation::Update(Action::Poll),
        }
    }
    #[test]
    fn full_output_retains_exact_ready_and_flush_payloads_in_completion_order() {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(Reply::Flushed(999, true)).unwrap();
        let mut queue = CriticalReplies::new();
        queue.retain(Reply::Ready(Completion {
            work: work(),
            result: Err("not an approval".into()),
        }));
        queue.retain(Reply::Flushed(8, false));
        queue.retain(Reply::Flushed(9, true));
        queue.poll(&tx);
        assert_eq!(queue.len(), 3);
        assert!(matches!(rx.recv().unwrap(), Reply::Flushed(999, true)));
        queue.poll(&tx);
        match rx.recv().unwrap() {
            Reply::Ready(done) => {
                assert_eq!(done.work, work());
                assert_eq!(done.result.unwrap_err(), "not an approval");
            }
            _ => panic!("a full channel changed completion order"),
        }
        queue.poll(&tx);
        assert!(matches!(rx.recv().unwrap(), Reply::Flushed(8, false)));
        queue.poll(&tx);
        assert!(matches!(rx.recv().unwrap(), Reply::Flushed(9, true)));
        assert_eq!(queue.len(), 0);
    }
    #[test]
    fn intake_budget_reserves_the_last_parallel_batch_without_unbounded_growth() {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(Reply::Flushed(999, true)).unwrap();
        let mut queue = CriticalReplies::new();
        for n in 0..MAX_JOBS - ACCESS_READ_BATCH {
            queue.retain(Reply::Flushed(n as u64, true));
        }
        assert!(queue.can_take_job());
        for n in 0..ACCESS_READ_BATCH {
            queue.retain(Reply::Flushed(n as u64, false));
        }
        queue.poll(&tx);
        assert_eq!(queue.len(), MAX_JOBS);
        assert!(!queue.can_take_job());
        rx.recv().unwrap();
        for _ in 0..ACCESS_READ_BATCH {
            queue.poll(&tx);
            rx.recv().unwrap();
        }
        assert!(queue.can_take_job());
    }
    #[test]
    fn receiver_disconnect_releases_the_abandoned_backlog_without_a_fake_ack() {
        let (tx, rx) = mpsc::sync_channel(1);
        let mut queue = CriticalReplies::new();
        queue.retain(Reply::Flushed(1, false));
        drop(rx);
        queue.poll(&tx);
        assert!(queue.receiver_closed());
        assert_eq!(queue.len(), 0);
        queue.retain(Reply::Flushed(2, true));
        assert_eq!(queue.len(), 0);
        assert!(queue.can_take_job());
    }
}
