use ffmpeg_sys_next::{av_packet_clone, av_read_frame, AVFormatContext, AVPacket};
use crate::ffmpeg::error::Error;

pub struct Packet {
    pointer: *mut AVPacket,
    pub serial: u32,
    pub id: usize
}

impl Packet {
    pub fn new(serial: u32, id: usize) -> Self {
        unsafe {
            let packet = ffmpeg_sys_next::av_packet_alloc();
            if packet.is_null() {
                panic!("av_packet_alloc fail");
            }
            Self {
                pointer: packet,
                serial,
                id
            }
        }
    }

    pub(super) fn read_from(&mut self, context: *mut AVFormatContext) -> Option<Error> {
        unsafe {
            let result = av_read_frame(context, self.pointer);
            if result < 0 {
                return Some(Error::from_code(result))
            }
            None
        }
    }

    pub fn duration(&self) -> i64 {
        unsafe {
            (*self.pointer).duration
        }
    }

    pub fn pts(&self) -> i64 {
        unsafe {
            (*self.pointer).pts
        }
    }

    pub fn stream_index(&self) -> i32 {
        unsafe {
            (*self.pointer).stream_index
        }
    }

    pub fn flags(&self) -> i32 {
        unsafe {
            (*self.pointer).flags
        }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe {
            ffmpeg_sys_next::av_packet_free(&mut self.pointer);
        }
    }
}

impl Clone for Packet {
    fn clone(&self) -> Self {
        unsafe {
            let clone = av_packet_clone(self.pointer);
            if clone.is_null() {
                panic!("av_packet_clone fail");
            }
            Packet {
                pointer: clone,
                serial: self.serial,
                id: self.id
            }
        }
    }
}