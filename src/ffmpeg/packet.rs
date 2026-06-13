use crate::ffmpeg::error::Error;
use ffmpeg_sys_next::{av_new_packet, AV_PKT_FLAG_KEY};
use ffmpeg_sys_next::av_packet_alloc;
use ffmpeg_sys_next::av_packet_clone;
use ffmpeg_sys_next::av_packet_new_side_data;
use ffmpeg_sys_next::av_read_frame;
use ffmpeg_sys_next::AVFormatContext;
use ffmpeg_sys_next::AVPacket;
use ffmpeg_sys_next::AVPacketSideDataType;

#[derive(Clone)]
pub struct ByteBuffer {
    buffer: Vec<u8>,
    read_index: usize,
}

impl ByteBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
            read_index: 0,
        }
    }

    pub fn read(&mut self, dest: &mut [u8]) -> Option<()> {
        if self.remaining() < dest.len() {
            return None;
        }

        let end = self.read_index + dest.len();

        dest.copy_from_slice(&self.buffer[self.read_index..end]);
        self.read_index = end;

        Some(())
    }

    pub fn read_ptr(&mut self, size: usize) -> Option<*mut u8> {
        if self.remaining() < size {
            return None;
        }
        let last = self.read_index;
        self.read_index = last + size;
        unsafe { Some(self.buffer.as_mut_ptr().add(last)) }
    }
    
    pub fn read_ser<T: Serializable>(&mut self) -> Option<T> {
        T::deserialize(self)
    }
    
    pub fn write_ser<T: Serializable>(&mut self, obj: &T) {
        obj.serialize(self);
    }

    pub fn write_zero<T: Sized>(&mut self) {
        for _ in 0..size_of::<T>() {
            self.buffer.push(0);
        }
    }

    pub fn read_zero<T: Sized>(&mut self) {
        self.read_index += size_of::<T>();
    }
    
    pub fn len(&self) -> usize {
        self.buffer.len()
    }
    
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    pub fn write(&mut self, src: &[u8]) {
        self.buffer.extend_from_slice(src);
    }

    pub fn remaining(&self) -> usize {
        self.buffer.len() - self.read_index
    }

    pub fn internal(&self) -> &[u8] {
        &self.buffer[..]
    }

    pub fn internal_mut(&mut self, size: usize) -> &mut [u8] {
        if self.remaining() < size {
            for _ in self.read_index..self.read_index + size {
                self.buffer.push(0);
            }
        }
        &mut self.buffer[self.read_index..size]
    }

    pub fn crc_32(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(self.internal());
        hasher.finalize()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.read_index = 0;
    }
}

pub trait Serializable {
    fn serialize(&self, buffer: &mut ByteBuffer);
    
    fn deserialize(buffer: &mut ByteBuffer) -> Option<Self> where Self: Sized;
}

macro_rules! impl_byte_serializable {
    ($($t:ty),* $(,)?) => {
        $(
            impl Serializable for $t {
                fn serialize(&self, buffer: &mut ByteBuffer) {
                    buffer.write(&<$t>::to_le_bytes(*self));
                }

                fn deserialize(buffer: &mut ByteBuffer) -> Option<Self> where Self: Sized {
                    let mut arr = [0u8; size_of::<$t>()];
                    buffer.read(&mut arr)?;
                    Some(<$t>::from_le_bytes(arr))
                }
            }
        )*
    };
}

impl_byte_serializable!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

impl Serializable for bool {
    fn serialize(&self, buffer: &mut ByteBuffer) {
        let val = if *self { 1u8 } else { 0u8 };
        buffer.write_ser(&val);
    }
    fn deserialize(buffer: &mut ByteBuffer) -> Option<Self> {
        buffer.read_ser::<u8>().map(|v| v == 1)
    }
}

impl Serializable for usize {
    fn serialize(&self, buffer: &mut ByteBuffer) {
        buffer.write_ser(&(*self as u64));
    }
    fn deserialize(buffer: &mut ByteBuffer) -> Option<Self> {
        let num = buffer.read_ser::<u64>()? as usize;
        Some(num)
    }
}

pub struct Packet {
    pub(super) pointer: *mut AVPacket,
    pub serial: u32,
    pub id: usize
}

unsafe impl Send for Packet {}
unsafe impl Sync for Packet {}

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

    pub fn from_raw(serial: u32, id: usize, ptr: *mut AVPacket) -> Self {
        unsafe {
            Self {
                pointer: ptr,
                serial,
                id
            }
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

impl Serializable for AVPacketSideDataType {
    fn serialize(&self, buffer: &mut ByteBuffer) {
        buffer.write_ser(&(*self as u32))
    }

    fn deserialize(buffer: &mut ByteBuffer) -> Option<Self>
    where
        Self: Sized
    {
        let n: u32 = buffer.read_ser()?;
        Some(unsafe { std::mem::transmute(n) })
    }
}

impl Serializable for Packet {
    fn serialize(&self, buffer: &mut ByteBuffer) {
        unsafe {
            buffer.write_ser(&self.serial);
            buffer.write_ser(&(self.id as u64));

            let packet = &*self.pointer;
            buffer.write_ser(&packet.size);

            // === fields ===
            buffer.write_ser(&packet.pts);
            buffer.write_ser(&packet.dts);
            buffer.write_ser(&packet.duration);
            buffer.write_ser(&packet.stream_index);
            buffer.write_ser(&packet.flags);
            buffer.write_ser(&packet.pos);
            // === === === ===

            let data = std::slice::from_raw_parts(packet.data, packet.size as usize);
            buffer.write(data);

            buffer.write_ser(&packet.side_data_elems);
            for i in 0..packet.side_data_elems as usize {
                let side_data = &*packet.side_data.add(i);
                let size = side_data.size;
                buffer.write_ser(&side_data.type_);
                buffer.write_ser(&(size as u32));
                let data = std::slice::from_raw_parts(side_data.data, size);
                buffer.write(data);
            }
        }
    }

    fn deserialize(buffer: &mut ByteBuffer) -> Option<Self> {
        unsafe {
            let serial = buffer.read_ser()?;
            let id = buffer.read_ser::<u64>()? as usize;
            let size: i32 = buffer.read_ser()?;
            let packet = &mut *av_packet_alloc();
            av_new_packet(packet, size);
            packet.pts = buffer.read_ser()?;
            packet.dts = buffer.read_ser()?;
            packet.duration = buffer.read_ser()?;
            packet.stream_index = buffer.read_ser()?;
            packet.flags = buffer.read_ser()?;
            packet.pos = buffer.read_ser()?;

            let data = std::slice::from_raw_parts_mut(packet.data, size as usize);
            buffer.read(data)?;

            let elems: i32 = buffer.read_ser()?;
            for _ in 0..elems as usize {
                let typ = buffer.read_ser()?;
                let size = buffer.read_ser::<u32>()? as usize;
                let side_data = av_packet_new_side_data(packet, typ, size);
                let side_data = std::slice::from_raw_parts_mut(side_data, size);
                buffer.read(side_data)?;
            }
            Some(Packet::from_raw(serial, id, packet))
        }
    }
}