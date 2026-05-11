use std::ffi::CString;
use std::str::FromStr;
use ffmpeg_sys_next::{av_dict_set, AVDictionary};

pub fn convert_options(options: Vec<(&str, &str)>) -> *mut AVDictionary {
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