use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use crate::ffmpeg;

pub struct AtomicF64(AtomicU64);

impl AtomicF64 {
    pub fn new(v: f64) -> Self {
        AtomicF64(AtomicU64::new(v.to_bits()))
    }

    pub fn load(&self, ordering: Ordering) -> f64 {
        f64::from_bits(self.0.load(ordering))
    }

    pub fn store(&self, val: f64, ordering: Ordering) {
        self.0.store(val.to_bits(), ordering);
    }
}

pub struct AtomicInstant(AtomicU64);

impl AtomicInstant {
    pub fn now() -> Self {
        AtomicInstant(AtomicU64::new(ffmpeg::current_time()))
    }

    pub fn elapsed(&self, ordering: Ordering) -> Duration {
        let current = ffmpeg::current_time();
        let last_update = self.0.load(ordering);
        let diff = if last_update > current {
            0
        } else {
            current - last_update
        };
        Duration::from_micros(diff)
    }

    pub fn set_now(&self, ordering: Ordering) {
        let now = ffmpeg::current_time();
        self.0.store(now, ordering);
    }
}

pub trait Clock {
    fn serial(&self) -> u32;
    fn pts(&self) -> f64;
    fn pts_interpolated(&self) -> f64;
    fn set_seek_flag(&self, seek: f64);
    fn is_ext(&self) -> bool;
    fn sync_ext(&self, pts: f64);
}

pub struct ExtClock {
    pts: f64,
    last_update: Instant,
}