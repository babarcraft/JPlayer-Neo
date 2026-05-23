use std::ffi::CString;
use std::str::FromStr;
use ffmpeg_sys_next::{av_dict_set, AVDictionary};

pub fn convert_options(options: &[(&str, &str)]) -> *mut AVDictionary {
    let mut dict: *mut AVDictionary = std::ptr::null_mut();
    for (key, value) in options {
        unsafe {
            let key = CString::from_str(key).unwrap();
            let value = CString::from_str(value).unwrap();
            
            av_dict_set(&mut dict, key.as_ptr(), value.as_ptr(), 0);
        }
    }
    
    dict
}

pub fn bt709() -> [[f32; 3]; 3] {
    [
        [1.0,       1.0,       1.0],
        [0.0,      -0.187324,  1.8556],
        [1.5748,   -0.468124,  0.0],
    ]
}

pub fn bt601() -> [[f32; 3]; 3] {
    [
        [1.164,  0.000,  1.596],
        [1.164, -0.391, -0.813],
        [1.164,  2.018,  0.000],
    ]
}

pub fn bt2020() -> [[f32; 3]; 3] {
    [
        [1.164,  0.000,  1.678],
        [1.164, -0.187, -0.650],
        [1.164,  2.141,  0.000],
    ]
}

pub fn rgb() -> [[f32; 3]; 3] {
    [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ]
}

pub fn fallback() -> [[f32; 3]; 3] {
    bt709()
}