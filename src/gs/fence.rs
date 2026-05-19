use std::time::Duration;
use gl::types::GLsync;

pub struct Fence {
    handle: Option<GLsync>
}

impl Fence {
    pub fn new() -> Self {
        Self {
            handle: None,
        }
    }

    pub fn check_done(&mut self, timeout: Option<Duration>) -> bool {
        let handle = match self.handle {
            Some(handle) => handle,
            None => return true,
        };
        unsafe {
            let result = gl::ClientWaitSync(handle, 0, timeout.unwrap_or(Duration::ZERO).as_millis() as u64);
            match result {
                gl::ALREADY_SIGNALED | gl::CONDITION_SATISFIED => {
                    gl::DeleteSync(handle);
                    self.handle = None;
                    true
                }
                _ => false
            }
        }
    }

    pub fn place(&mut self) {
        if let Some(handle) = self.handle {
            unsafe {
                gl::DeleteSync(handle);
            }
            self.handle = None;
        }
        let handle = unsafe { gl::FenceSync(gl::SYNC_GPU_COMMANDS_COMPLETE, 0) };
        self.handle = Some(handle);
    }
}