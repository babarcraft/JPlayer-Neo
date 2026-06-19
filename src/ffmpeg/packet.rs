use std::sync::atomic::{AtomicUsize, Ordering};
use crate::ffmpeg::error::Error;
use ffmpeg_sys_next::{av_packet_alloc, av_packet_side_data_new, AVPacketSideData, AVPacketSideDataType};
use ffmpeg_sys_next::av_packet_clone;
use ffmpeg_sys_next::av_read_frame;
use ffmpeg_sys_next::AVFormatContext;
use ffmpeg_sys_next::AVPacket;
use ffmpeg_sys_next::{av_new_packet, AV_PKT_FLAG_KEY};

pub static PACKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct Packet {
    pub pointer: *mut AVPacket,
    pub serial: u32,
    pub id: usize
}

unsafe impl Send for Packet {}
unsafe impl Sync for Packet {}

impl Packet {
    
    pub fn new(serial: u32, id: usize) -> Self {
        unsafe {
            PACKET_COUNTER.fetch_add(1, Ordering::SeqCst);
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

    pub fn with_size(serial: u32, id: usize, size: usize) -> Self {
        unsafe {
            PACKET_COUNTER.fetch_add(1, Ordering::SeqCst);
            let packet = av_packet_alloc();
            if packet.is_null() {
                panic!("av_packet_alloc fail");
            }
            let res = av_new_packet(packet, size as i32);
            if res < 0 {
                panic!("av_new_packet fail");
            }
            Self {
                pointer: packet,
                serial,
                id
            }
        }
    }

    pub fn inner_ref(&self) -> &mut AVPacket {
        unsafe {
            &mut *self.pointer
        }
    }

    pub fn payload(&self) -> &mut [u8] {
        unsafe {
            let inner = self.inner_ref();
            std::slice::from_raw_parts_mut(inner.data, inner.size as usize)
        }
    }

    pub fn add_side_data(&self, size: usize, typ: AVPacketSideDataType) -> Option<&mut [u8]> {
        unsafe {
            let inner = self.inner_ref();
            let data = av_packet_side_data_new(&mut inner.side_data, &mut inner.side_data_elems, typ, size, 0);
            Some(data).take_if(|p| !p.is_null())
                .map(|p| &mut *p)
                .map(|data| std::slice::from_raw_parts_mut(data.data, data.size))
        }
    }

    pub(super) fn read_from(&mut self, context: *mut AVFormatContext) -> Result<(), Error> {
        unsafe {
            let result = av_read_frame(context, self.pointer);
            if result < 0 {
                return Err(Error::from_code(result))
            }
            Ok(())
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

    pub fn is_key(&self) -> bool {
        self.flags() & AV_PKT_FLAG_KEY > 0
    }
}

impl PartialEq for Packet {
    fn eq(&self, other: &Self) -> bool {
        self.pts() == other.pts() && self.flags() == other.flags() && self.stream_index() == other.stream_index()
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe {
            PACKET_COUNTER.fetch_sub(1, Ordering::SeqCst);
            ffmpeg_sys_next::av_packet_free(&mut self.pointer);
        }
    }
}

impl Clone for Packet {
    fn clone(&self) -> Self {
        unsafe {
            PACKET_COUNTER.fetch_add(1, Ordering::SeqCst);
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
