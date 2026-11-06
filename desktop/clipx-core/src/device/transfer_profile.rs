use std::time::{Duration, Instant};

#[allow(unused)]
#[derive(Debug, Default, Clone, Copy)]
pub struct OpStat {
    count: u64,
    total: Duration,
    max: Duration,
}

impl OpStat {
    pub fn record(&mut self, d: Duration) {
        self.count += 1;
        self.total += d;
        self.max = self.max.max(d);
    }

    pub fn count(&self) -> u64 {
        self.count
    }
    pub fn avg_us(&self) -> u64 {
        if self.count == 0 {
            0
        } else {
            (self.total.as_micros() / self.count as u128) as u64
        }
    }

    pub fn max_us(&self) -> u64 {
        self.max.as_micros() as u64
    }
}

pub struct SenderProfile {
    started_at: Instant,
    pub bytes: u64,
    pub read: OpStat,
    pub hash: OpStat,
    pub queue: OpStat,
    pub ack_wait: OpStat,
    pub cycle: OpStat,
    pub notify: OpStat,
}

impl SenderProfile {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            bytes: 0,
            read: Default::default(),
            hash: Default::default(),
            queue: Default::default(),
            ack_wait: Default::default(),
            cycle: Default::default(),
            notify: Default::default(),
        }
    }
    pub fn log_summary(&self, file_id: &str, outcome: &str) {
        tracing::debug!(
            %file_id, outcome, chunks = self.cycle.count(), bytes = self.bytes,
            elapsed_ms = self.started_at.elapsed().as_millis() as u64,
            read_avg_us = self.read.avg_us(), read_max_us = self.read.max_us(),
            hash_avg_us = self.hash.avg_us(), hash_max_us = self.hash.max_us(),
            queue_avg_us = self.queue.avg_us(), queue_max_us = self.queue.max_us(),
            ack_avg_us = self.ack_wait.avg_us(), ack_max_us = self.ack_wait.max_us(),
            cycle_avg_us = self.cycle.avg_us(), cycle_max_us = self.cycle.max_us(),
            notify_avg_us = self.notify.avg_us(), notify_max_us = self.notify.max_us(),
            "sender transfer profile"
        );
    }
}

pub struct ReceiverProfile {
    started_at: Instant,
    pub bytes: u64,
    pub hash: OpStat,
    pub seek: OpStat,
    pub write: OpStat,
    pub sync: OpStat,
    pub resume_state: OpStat,
    pub ack_queue: OpStat,
    pub handler: OpStat,
}

impl ReceiverProfile {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            bytes: 0,
            hash: Default::default(),
            seek: Default::default(),
            write: Default::default(),
            sync: Default::default(),
            resume_state: Default::default(),
            ack_queue: Default::default(),
            handler: Default::default(),
        }
    }
    pub fn log_summary(&self, file_id: &str, outcome: &str) {
        tracing::debug!(
            %file_id, outcome, chunks = self.handler.count(), bytes = self.bytes,
            elapsed_ms = self.started_at.elapsed().as_millis() as u64,
            hash_avg_us = self.hash.avg_us(), hash_max_us = self.hash.max_us(),
            seek_avg_us = self.seek.avg_us(), write_avg_us = self.write.avg_us(),
            write_max_us = self.write.max_us(),
            sync_avg_us = self.sync.avg_us(), sync_max_us = self.sync.max_us(),
            resume_avg_us = self.resume_state.avg_us(), resume_max_us = self.resume_state.max_us(),
            ack_queue_avg_us = self.ack_queue.avg_us(),
            handler_avg_us = self.handler.avg_us(), handler_max_us = self.handler.max_us(),
            "receiver transfer profile"
        );
    }
}
