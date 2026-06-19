use std::ffi::{c_char, CStr};
use ffmpeg_sys_next::{av_make_error_string, av_strerror, AVERROR, AVERROR_EOF, AVERROR_EXIT, EOF};

#[derive(Debug)]
pub struct Error {
    message: String,
    code: i32,
}

impl Error {
    pub fn from_code(code: i32) -> Error {
        unsafe {
            let mut buffer: [c_char; 128] = [0 as c_char; 128];
            let result = av_make_error_string(buffer.as_mut_ptr() as *mut c_char, buffer.len(), code);
            if result.is_null() {
                panic!("Invalid error code!")
            }
            let error_string = CStr::from_ptr(buffer.as_ptr()).to_string_lossy().to_string();
            Error {
                message: error_string,
                code,
            }
        }
    }
    
    pub fn is_eof(&self) -> bool {
        self.code == AVERROR_EOF
    }

    pub fn is_exit(&self) -> bool {
        self.code == AVERROR_EXIT
    }
}