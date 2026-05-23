use std::time::Instant;

pub trait Clock {
    fn serial(&self) -> u32;
    fn pts(&self) -> f64;
    fn pts_interpolated(&self) -> f64;
    fn set_seek_flag(&self, seek: f64);
}

pub struct GenClock {
    pts: Option<f64>,
    last_update: Option<Instant>,
    serial: Option<u32>
}

impl GenClock {
    pub fn new() -> Self {
        Self {
            pts: None,
            last_update: None,
            serial: None
        }
    }

    pub fn update(&mut self, pts: f64, serial: u32) {
        self.pts = Some(pts);
        self.last_update = Some(Instant::now());
        self.serial = Some(serial);
    }

    pub fn clear(&mut self) {
        self.pts = None;
        self.last_update = None;
    }

    pub fn pts_interpolated(&self) -> Option<f64> {
        if let Some(pts) = self.pts {
            if let Some(last_update) = self.last_update {
                return Some(pts + last_update.elapsed().as_secs_f64());
            }
        }
        None
    }

    pub fn pts(&self) -> Option<f64> {
        self.pts
    }

    pub fn serial(&self) -> Option<u32> {
        self.serial
    }
}