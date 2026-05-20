use std::time::Instant;

pub struct Clock {
    pts: Option<f64>,
    last_update: Option<Instant>,
    serial: Option<u32>
}

impl Clock {
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

    pub fn pts_interpolated(&self) -> Option<f64> {
        if let Some(pts) = self.pts {
            if let Some(last_update) = self.last_update {
                return Some(pts + last_update.elapsed().as_secs_f64());
            }
        }
        None
    }

    pub fn serial(&self) -> Option<u32> {
        self.serial
    }
}