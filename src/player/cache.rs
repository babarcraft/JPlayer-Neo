use crate::ffmpeg;
use crate::ffmpeg::input::{CodecParameters, Input, Stream, StreamMetaData, StreamType};
use crate::ffmpeg::packet::Packet;
use crate::player::clock::AtomicF64;
use crate::player::input::InputWorkerNotifier;
use ffmpeg_sys_next::{av_init_packet, av_malloc_array, av_mallocz, av_packet_new_side_data, av_packet_side_data_new, avcodec_parameters_alloc, AVChannelCustom, AVChannelOrder, AVCodecParameters, AVPacketSideDataType, AV_INPUT_BUFFER_PADDING_SIZE, AV_NOPTS_VALUE, AV_PKT_FLAG_KEY};
use std::ffi::c_char;
use std::fs::{File, OpenOptions};
use std::intrinsics::transmute;
use std::io::{ErrorKind, Write};
use std::mem::replace;
use std::ops::Range;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, RwLock, RwLockReadGuard};
use std::thread::JoinHandle;

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

    pub fn read_frame(&mut self) -> Option<&[u8]> {
        let len: usize = self.read_ser()?;
        Some(&self.buffer[self.read_index..self.read_index + len])
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

    pub fn write_frame(&mut self, src: &[u8]) {
        self.write_ser(&src.len());
        self.write(src);
    }

    pub fn remaining(&self) -> usize {
        self.buffer.len() - self.read_index
    }

    pub fn internal(&self) -> &[u8] {
        &self.buffer[..]
    }

    pub fn internal_mut(&mut self, size: usize) -> &mut [u8] {
        if self.len() < size {
            self.buffer.resize(size, 0);
        }
        &mut self.buffer[..size]
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

impl Serializable for Packet {
    fn serialize(&self, buffer: &mut ByteBuffer) {
        unsafe {
            buffer.write_ser(&self.serial);
            buffer.write_ser(&(self.id as u64));

            let packet = self.inner_ref();
            buffer.write_ser(&packet.size);

            // === fields ===
            buffer.write_ser(&packet.pts);
            buffer.write_ser(&packet.dts);
            buffer.write_ser(&packet.duration);
            buffer.write_ser(&packet.stream_index);
            buffer.write_ser(&packet.flags);
            // === === === ===

            let data = std::slice::from_raw_parts(packet.data, packet.size as usize);
            buffer.write(data);

            buffer.write_ser(&packet.side_data_elems);
            for i in 0..packet.side_data_elems as usize {
                let side_data = &*packet.side_data.add(i);
                let size = side_data.size;
                buffer.write_ser(&(side_data.type_ as u32));
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
            let packet_out = Packet::with_size(serial, id, size as usize);
            let packet = packet_out.inner_ref();
            packet.pts = buffer.read_ser()?;
            packet.dts = buffer.read_ser()?;
            packet.duration = buffer.read_ser()?;
            packet.stream_index = buffer.read_ser()?;
            packet.flags = buffer.read_ser()?;
            packet.pos = 0;
            packet.opaque = null_mut();
            packet.opaque_ref = null_mut();

            buffer.read(packet_out.payload())?;

            let elems: i32 = buffer.read_ser()?;
            for _ in 0..elems as usize {
                let typ: AVPacketSideDataType = transmute(buffer.read_ser::<u32>()?);
                let size = buffer.read_ser::<u32>()? as usize;
                let side_data = packet_out.add_side_data(size, typ)?;
                buffer.read(side_data)?;
            }
            Some(packet_out)
        }
    }
}

impl Serializable for CodecParameters {
    fn serialize(&self, buffer: &mut ByteBuffer) {
        let inner = self.inner_mut();
        buffer.write_ser(&(inner.codec_type as i32));
        buffer.write_ser(&(inner.codec_id as i32));
        buffer.write_ser(&inner.codec_tag);
        unsafe {
            if let Some(extra_data) = self.extra_data() {
                buffer.write_ser(&extra_data.len());
                buffer.write(extra_data);
            } else {
                buffer.write_ser(&0usize);
            }

            buffer.write_ser(&inner.nb_coded_side_data);
            if let Some(side_data) = self.coded_side_data() {
                buffer.write_ser(&(side_data.len() as u32));
                for side_data in side_data {
                    buffer.write_ser(&side_data.size);
                    buffer.write_ser(&(side_data.type_ as i32));
                    buffer.write(std::slice::from_raw_parts(side_data.data, side_data.size));
                }
            } else {
                buffer.write_ser(&0u32);
            }

            buffer.write_ser(&inner.format);
            buffer.write_ser(&inner.bit_rate);
            buffer.write_ser(&inner.bits_per_coded_sample);
            buffer.write_ser(&inner.bits_per_raw_sample);
            buffer.write_ser(&inner.profile);
            buffer.write_ser(&inner.level);
            buffer.write_ser(&inner.width);
            buffer.write_ser(&inner.height);
            buffer.write_ser(&inner.sample_aspect_ratio.num);
            buffer.write_ser(&inner.sample_aspect_ratio.den);
            buffer.write_ser(&inner.framerate.num);
            buffer.write_ser(&inner.framerate.den);
            buffer.write_ser(&(inner.field_order as i32));
            buffer.write_ser(&(inner.color_range as i32));
            buffer.write_ser(&(inner.color_primaries as i32));
            buffer.write_ser(&(inner.color_trc as i32));
            buffer.write_ser(&(inner.color_space as i32));
            buffer.write_ser(&(inner.chroma_location as i32));
            buffer.write_ser(&inner.video_delay);

            buffer.write_ser(&inner.ch_layout.nb_channels);
            buffer.write_ser(&(inner.ch_layout.order as i32));
            match &inner.ch_layout.order {
                AVChannelOrder::AV_CHANNEL_ORDER_NATIVE => {
                    buffer.write_ser(&inner.ch_layout.u.mask);
                }
                AVChannelOrder::AV_CHANNEL_ORDER_CUSTOM => {
                    let channels = std::slice::from_raw_parts(inner.ch_layout.u.map, inner.ch_layout.nb_channels as usize);
                    buffer.write_ser(&channels.len());
                    for channel in channels {
                        let ptr = channel.name.as_ptr();
                        let slice = std::slice::from_raw_parts(ptr as *const u8, channel.name.len() * size_of::<c_char>());
                        buffer.write(slice);
                        buffer.write_ser(&(channel.id as i32));
                    }
                }
                _ => {}
            }

            buffer.write_ser(&inner.sample_rate);
            buffer.write_ser(&inner.block_align);
            buffer.write_ser(&inner.frame_size);
            buffer.write_ser(&inner.initial_padding);
            buffer.write_ser(&inner.trailing_padding);
            buffer.write_ser(&inner.seek_preroll);
            buffer.write_ser(&(inner.alpha_mode as i32));
        }
    }

    fn deserialize(buffer: &mut ByteBuffer) -> Option<Self> {
        unsafe {
            let codec = CodecParameters::new();
            let inner = codec.inner_mut();
            inner.codec_type = transmute(buffer.read_ser::<i32>()?);
            inner.codec_id = transmute(buffer.read_ser::<i32>()?);
            inner.codec_tag = buffer.read_ser()?;

            let extra_data_len = buffer.read_ser::<usize>()?;
            if extra_data_len > 0 {
                codec.allocate_extra_data(extra_data_len);
                if let Some(extra_data) = codec.extra_data() {
                    buffer.read(extra_data)?;
                }
            }

            let nb_sides = buffer.read_ser::<i32>()?;
            for _ in 0..nb_sides as usize {
                let size = buffer.read_ser::<usize>()?;
                let typ = transmute(buffer.read_ser::<i32>()?);
                let data = codec.add_coded_side_data(typ, size);
                buffer.read(data);
            }

            inner.format = buffer.read_ser()?;
            inner.bit_rate = buffer.read_ser()?;
            inner.bits_per_coded_sample = buffer.read_ser()?;
            inner.bits_per_raw_sample = buffer.read_ser()?;
            inner.profile = buffer.read_ser()?;
            inner.level = buffer.read_ser()?;
            inner.width = buffer.read_ser()?;
            inner.height = buffer.read_ser()?;
            inner.sample_aspect_ratio.num = buffer.read_ser()?;
            inner.sample_aspect_ratio.den = buffer.read_ser()?;
            inner.framerate.num = buffer.read_ser()?;
            inner.framerate.den = buffer.read_ser()?;
            inner.field_order = transmute(buffer.read_ser::<i32>()?);
            inner.color_range = transmute(buffer.read_ser::<i32>()?);
            inner.color_primaries = transmute(buffer.read_ser::<i32>()?);
            inner.color_trc = transmute(buffer.read_ser::<i32>()?);
            inner.color_space = transmute(buffer.read_ser::<i32>()?);
            inner.chroma_location = transmute(buffer.read_ser::<i32>()?);
            inner.video_delay = buffer.read_ser()?;

            inner.ch_layout.nb_channels = buffer.read_ser()?;
            inner.ch_layout.order = transmute(buffer.read_ser::<i32>()?);
            match &inner.ch_layout.order {
                AVChannelOrder::AV_CHANNEL_ORDER_NATIVE => {
                    inner.ch_layout.u.mask = buffer.read_ser()?;
                }
                AVChannelOrder::AV_CHANNEL_ORDER_CUSTOM => {
                    let len = buffer.read_ser::<usize>()?;
                    inner.ch_layout.u.map = av_malloc_array(len, size_of::<AVChannelCustom>()) as *mut AVChannelCustom;
                    let channels = std::slice::from_raw_parts_mut(inner.ch_layout.u.map, len);
                    for channel in channels {
                        let ptr = channel.name.as_ptr();
                        let slice = std::slice::from_raw_parts_mut(ptr as *mut u8, channel.name.len() * size_of::<c_char>());
                        buffer.read(slice);
                        channel.id = transmute(buffer.read_ser::<i32>()?);
                        channel.opaque = null_mut();
                    }
                }
                _ => {}
            }
            inner.ch_layout.opaque = null_mut();

            inner.sample_rate = buffer.read_ser()?;
            inner.block_align = buffer.read_ser()?;
            inner.frame_size = buffer.read_ser()?;
            inner.initial_padding = buffer.read_ser()?;
            inner.trailing_padding = buffer.read_ser()?;
            inner.seek_preroll = buffer.read_ser()?;
            inner.alpha_mode = transmute(buffer.read_ser::<i32>()?);

            Some(codec)
        }
    }
}

enum SegmentLine {
    Packet(Packet),
    Jump(Option<u64>),
    Stream(Stream),
    Seal
}

impl Serializable for SegmentLine {
    fn serialize(&self, buffer: &mut ByteBuffer) {
        match self {
            SegmentLine::Packet(packet) => {
                buffer.write_ser(&0u8);
                buffer.write_ser(packet)
            },
            SegmentLine::Jump(jump) => {
                buffer.write_ser(&1u8);
                if let Some(jump) = jump {
                    buffer.write_ser(&true);
                    buffer.write_ser(jump)
                } else {
                    buffer.write_ser(&false);
                    buffer.write_zero::<u64>()
                }
            }
            SegmentLine::Stream(stream) => {
                buffer.write_ser(&2u8);
                buffer.write_ser(&stream.index);
                buffer.write_ser(&stream.timebase);
                buffer.write_ser(&stream.duration);
                buffer.write_ser(&stream.start_time);
                buffer.write_ser(&(stream.stream_type as u8));
                buffer.write_ser(stream.codec());
            }
            SegmentLine::Seal => {
                buffer.write_ser(&3u8);
            }
        }
    }
    fn deserialize(buffer: &mut ByteBuffer) -> Option<Self> {
        let code = buffer.read_ser::<u8>()?;
        match code {
            0 => Some(SegmentLine::Packet(buffer.read_ser()?)),
            1 => {
                let is_some: bool = buffer.read_ser()?;
                let val = if is_some {
                    Some(buffer.read_ser()?)
                } else {
                    buffer.read_zero::<u64>();
                    None
                };
                Some(SegmentLine::Jump(val))
            },
            2 => {
                Some(SegmentLine::Stream(Stream {
                    index: buffer.read_ser()?,
                    timebase: buffer.read_ser()?,
                    duration: buffer.read_ser()?,
                    start_time: buffer.read_ser()?,
                    stream_type: unsafe { transmute(buffer.read_ser::<u8>()?) },
                    parameters: buffer.read_ser()?,
                }))
            }
            3 => Some(SegmentLine::Seal),
            _ => None
        }
    }
}

enum SegmentLineRead {
    Packet(i64, i32, i32),
    Jump(Option<u64>),
    Stream(Stream),
    Seal
}

impl Serializable for SegmentLineRead {
    fn serialize(&self, buffer: &mut ByteBuffer) {
        unimplemented!()
    }

    fn deserialize(buffer: &mut ByteBuffer) -> Option<Self> {
        let code = buffer.read_ser::<u8>()?;
        match code {
            0 => {
                buffer.read_ser::<u32>()?;
                buffer.read_ser::<u64>()?;
                let size: i32 = buffer.read_ser()?;
                let pts: i64 = buffer.read_ser()?;
                buffer.read_ser::<i64>()?;
                buffer.read_ser::<i64>()?;
                let stream_index: i32 = buffer.read_ser()?;
                let flags: i32 = buffer.read_ser()?;
                buffer.read_index += size as usize;

                let elems = buffer.read_ser::<i32>()?;
                for _ in 0..elems {
                    buffer.read_ser::<u32>()?;
                    let size = buffer.read_ser::<u32>()? as usize;
                    buffer.read_index += size;
                }

                Some(SegmentLineRead::Packet(pts, stream_index, flags))
            },
            1 => {
                let is_some: bool = buffer.read_ser()?;
                let val = if is_some {
                    Some(buffer.read_ser()?)
                } else {
                    buffer.read_zero::<u64>();
                    None
                };
                Some(SegmentLineRead::Jump(val))
            },
            2 => {
                Some(SegmentLineRead::Stream(Stream {
                    index: buffer.read_ser()?,
                    timebase: buffer.read_ser()?,
                    duration: buffer.read_ser()?,
                    start_time: buffer.read_ser()?,
                    stream_type: unsafe { transmute(buffer.read_ser::<u8>()?) },
                    parameters: buffer.read_ser()?,
                }))
            }
            3 => Some(SegmentLineRead::Seal),
            _ => None
        }
    }
}

pub struct CacheFile {
    file: Arc<File>,
    buffer: ByteBuffer,
    read_index: usize,
    write_index: Arc<AtomicUsize>,
}

impl CacheFile {
    pub fn new<T: AsRef<Path>>(file: T) -> Self {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(file)
            .unwrap();
        Self {
            file: Arc::new(file),
            buffer: ByteBuffer::new(1024),
            read_index: 0,
            write_index: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn open<T: AsRef<Path>>(path: T) -> Result<Self, std::io::Error> {
        let file = OpenOptions::new()
            .read(true)
            .open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            file: Arc::new(file),
            buffer: ByteBuffer::new(1024),
            read_index: 0,
            write_index: Arc::new(AtomicUsize::new(len as usize)),
        })
    }

    pub fn write_packet(&mut self, buffer: &ByteBuffer) -> Result<usize, std::io::Error> {
        let mut index = self.write_index.load(Ordering::SeqCst);
        self.buffer.clear();
        self.buffer.write_ser(&(buffer.len() as u32));
        self.buffer.write_ser(&buffer.crc_32());
        index += self.file.write_at(self.buffer.internal(), index as u64)?;
        index += self.file.write_at(buffer.internal(), index as u64)?;
        self.write_index.store(index, Ordering::SeqCst);
        Ok(index)
    }

    pub fn write_packet_at(&mut self, mut index: usize, buffer: &ByteBuffer) -> Result<usize, std::io::Error> {
        self.buffer.clear();
        self.buffer.write_ser(&(buffer.len() as u32));
        self.buffer.write_ser(&buffer.crc_32());
        index += self.file.write_at(self.buffer.internal(), index as u64)?;
        index += self.file.write_at(buffer.internal(), index as u64)?;
        Ok(index)
    }

    pub fn read_packet(&mut self, buffer: &mut ByteBuffer) -> Result<(), std::io::Error> {
        self.buffer.clear();
        {
            let dest = self.buffer.internal_mut(size_of::<u32>() * 2);
            self.file.read_at(dest, self.read_index as u64)?;
            self.read_index += dest.len();
        }
        let len = self.buffer.read_ser::<u32>().ok_or_else(||std::io::Error::new(ErrorKind::Other, "Could not read packet size."))?;
        let crc = self.buffer.read_ser::<u32>().ok_or_else(||std::io::Error::new(ErrorKind::Other, "Could not read packet crc."))?;

        self.buffer.clear();

        {
            let dest = buffer.internal_mut(len as usize);
            self.file.read_at(dest, self.read_index as u64)?;
            self.read_index += dest.len();
        }
        if crc != buffer.crc_32() {
            Err(std::io::Error::new(ErrorKind::Other, "File corrupted!"))
        } else {
            Ok(())
        }
    }

    pub fn read_packet_at(&mut self, mut index: usize, buffer: &mut ByteBuffer) -> Result<usize, std::io::Error> {
        self.buffer.clear();
        {
            let dest = self.buffer.internal_mut(size_of::<u32>() * 2);
            index += self.file.read_at(dest, index as u64)?;
        }
        let len = self.buffer.read_ser::<u32>().ok_or_else(||std::io::Error::new(ErrorKind::Other, "Could not read packet size."))?;
        let crc = self.buffer.read_ser::<u32>().ok_or_else(||std::io::Error::new(ErrorKind::Other, "Could not read packet crc."))?;

        self.buffer.clear();

        {
            let dest = buffer.internal_mut(len as usize);
            index += self.file.read_at(dest, index as u64)?;
        }
        if crc != buffer.crc_32() {
            Err(std::io::Error::new(ErrorKind::Other, "File corrupted!"))
        } else {
            Ok(index)
        }
    }
}

impl Clone for CacheFile {
    fn clone(&self) -> Self {
        Self {
            file: self.file.clone(),
            buffer: ByteBuffer::new(1024),
            read_index: self.read_index.clone(),
            write_index: self.write_index.clone()
        }
    }
}

#[derive(Clone)]
enum SegmentSyncStage {
    InitSeek(f64),
    SyncSeek(f64, Packet),
    SyncForward(f64, Packet),
}

#[derive(Clone)]
struct SeekPoint {
    pts: f64,
    stream: i32,
    offset: usize
}

impl PartialEq for SeekPoint {
    fn eq(&self, other: &Self) -> bool {
        self.pts.eq(&other.pts)
    }
}
impl PartialOrd for SeekPoint {
    fn partial_cmp(&self, other: &SeekPoint) -> Option<std::cmp::Ordering> {
        self.pts.partial_cmp(&other.pts)
    }
}
impl Eq for SeekPoint {}
impl Ord for SeekPoint {
    fn cmp(&self, other: &SeekPoint) -> std::cmp::Ordering {
        self.pts.partial_cmp(&other.pts).unwrap()
    }
}

struct Segment {
    file: CacheFile,
    buffer: ByteBuffer,

    begin: Arc<AtomicF64>,
    end: Arc<AtomicF64>,
    size: Arc<AtomicU64>,
    seek_table: Arc<RwLock<Vec<SeekPoint>>>,
    sealed: Arc<AtomicBool>,

    current_pts: Option<f64>,

    first_key: Option<u64>,
    last_key: Option<u64>,
    stream_meta: Vec<StreamMetaData>,
    preferred_stream: Option<i32>,
    empty_jump: Option<u64>,

    sync_stage: Option<SegmentSyncStage>,
}

impl Segment {

    fn new(file: CacheFile, stream_meta_data: Vec<StreamMetaData>) -> Self {
        let preferred_stream = stream_meta_data.iter()
            .find(|str| str.stream_type == StreamType::Video)
            .map(|str| str.index);
        Self {
            file,

            buffer: ByteBuffer::new(1024),
            begin: Arc::new(AtomicF64::new(f64::NAN)),
            end: Arc::new(AtomicF64::new(f64::NAN)),
            size: Arc::new(AtomicU64::new(0)),
            sealed: Arc::new(AtomicBool::new(false)),
            seek_table: Arc::new(RwLock::new(Vec::new())),

            stream_meta: stream_meta_data,
            preferred_stream,

            empty_jump: None,
            current_pts: None,
            last_key: None,
            first_key: None,
            sync_stage: None,
        }
    }

    fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::SeqCst)
    }

    fn set_sealed(&mut self) -> Result<(), std::io::Error> {
        self.sealed.store(true, Ordering::SeqCst);
        self.write_line(&SegmentLine::Seal)?;
        Ok(())
    }

    fn continue_from_end(&mut self, other: &mut Segment) -> Result<(), CacheError> {
        other.end().map_err(CacheError::WriteError)?;
        if let Some(jump_index) = self.empty_jump.take() {
            let end = self.file.write_index.load(Ordering::SeqCst);
            self.write_line_at(jump_index as usize, &SegmentLine::Jump(Some(end as u64))).map_err(CacheError::WriteError)?;
            if !self.is_sealed() {
                self.set_sync()?;
            }
        }

        Ok(())
    }

    fn write_line(&mut self, line: &SegmentLine) -> Result<(), std::io::Error> {
        self.buffer.clear();
        self.buffer.write_ser(line);
        let last = self.file.write_index.load(Ordering::SeqCst);
        let after = self.file.write_packet(&self.buffer)?;
        self.size.fetch_add((after - last) as u64, Ordering::Relaxed);
        Ok(())
    }

    fn read_line(&mut self) -> Result<SegmentLine, std::io::Error> {
        self.buffer.clear();
        self.file.read_packet(&mut self.buffer)?;
        self.buffer.read_ser::<SegmentLine>().ok_or_else(||
            std::io::Error::new(ErrorKind::Other, "Could not read line."))
    }

    fn update(&mut self, packet: &Packet) {
        let pts = packet.pts();
        let stream = &self.stream_meta[packet.stream_index() as usize];
        let stream_filter = self.preferred_stream.map(|p| p == packet.stream_index()).unwrap_or(true);
        if pts != AV_NOPTS_VALUE && packet.is_key() && stream_filter {
            let pts = (pts as f64) * stream.timebase;
            let begin = self.begin.load(Ordering::SeqCst);
            let end = self.end.load(Ordering::SeqCst);
            if pts < begin || begin.is_nan() {
                self.begin.store(pts, Ordering::SeqCst);
            }
            if end < pts || end.is_nan() {
                self.end.store(pts, Ordering::SeqCst);
            }
            let mut seek_table = self.seek_table.write().unwrap();
            let push = seek_table.last()
                .take_if(|point| pts - point.pts < 0.5)
                .is_none() || self.preferred_stream.is_some();
            let index = self.file.write_index.load(Ordering::SeqCst) as u64;
            self.first_key.get_or_insert(index);
            self.last_key = Some(index);
            if push {
                seek_table.push(SeekPoint {
                    pts,
                    stream: stream.index,
                    offset: index as usize,
                });
            }
        }
    }

    fn seek(&mut self, target: f64) -> bool {
        let begin = self.begin.load(Ordering::SeqCst);
        let end = self.end.load(Ordering::SeqCst);
        if target < begin || target > end || end.is_nan() || begin.is_nan() {
            return false;
        }

        let seek_table = self.seek_table.read().unwrap();
        let res = seek_table.binary_search_by(|a| a.pts.total_cmp(&target)).unwrap_or_else(|i| i);
        let point = &seek_table[res.max(1) - 1];
        self.file.read_index = point.offset;
        self.current_pts = Some(point.pts);
        true
    }

    fn write_packet(&mut self, packet: Packet) -> Result<(), std::io::Error> {
        self.update(&packet);
        self.write_line(&SegmentLine::Packet(packet))?;
        Ok(())
    }

    fn end(&mut self) -> Result<(), std::io::Error> {
        if self.is_sealed() { return Ok(()); }
        self.empty_jump = Some(self.file.write_index.load(Ordering::SeqCst) as u64);
        self.write_line(&SegmentLine::Jump(None))?;
        Ok(())
    }

    fn overlaps_with(&self, other: &Segment) -> bool {
        let this_begin = self.begin.load(Ordering::SeqCst);
        let this_end = self.end.load(Ordering::SeqCst);
        let other_begin = other.begin.load(Ordering::SeqCst);
        let other_end = other.end.load(Ordering::SeqCst);
        this_end > other_begin && this_begin < other_begin
    }

    fn read_packet_at(&mut self, offset: usize) -> Result<Option<(Packet, usize)>, std::io::Error> {
        self.buffer.clear();
        let after = self.file.read_packet_at(offset, &mut self.buffer)?;
        let line = self.buffer.read_ser::<SegmentLine>().ok_or_else(||
            std::io::Error::new(ErrorKind::Other, "Could not read line."))?;
        let packet = match line {
            SegmentLine::Packet(packet) => {
                Some(packet)
            }
            _ => None
        };
        Ok(packet.zip(Some(after)))
    }

    fn write_line_at(&mut self, offset: usize, line: &SegmentLine) -> Result<usize, std::io::Error> {
        self.buffer.clear();
        self.buffer.write_ser(line);
        let after = self.file.write_packet_at(offset, &self.buffer)?;
        Ok(after)
    }

    fn try_merge(&mut self, mut other: Segment) -> Result<(), CacheError> {
        let last_packet_pos = match self.last_key {
            Some(index) => index as usize,
            None => return Ok(())
        };
        let (last_packet, last_packet_after) = match self.read_packet_at(last_packet_pos).map_err(CacheError::ReadError)? {
            Some(packet) => packet,
            None => return Ok(()),
        };
        let last_size = last_packet_after - last_packet_pos;
        let mut current = match other.first_key {
            Some(index) => index as usize,
            None => return Ok(())
        };
        let mut found = false;
        while let Some((packet, after)) = other.read_packet_at(current).map_err(CacheError::ReadError)? {
            if packet == last_packet {
                found = true;
                break
            }
            current = after;
        }

        if found {
            // Here, we write a jump to the other segment's first packet
            self.write_line_at(last_packet_pos, &SegmentLine::Jump(Some(current as u64))).map_err(CacheError::WriteError)?;

            // Now, I need to write another jump, capture other's last packet and write it at 'last' index
            let other_last_pos = match other.last_key {
                Some(index) => index as usize,
                None => return Ok(())
            };
            let (packet, _) = match other.read_packet_at(other_last_pos).map_err(CacheError::ReadError)? {
                Some(packet) => packet,
                None => return Ok(()),
            };
            let write_index = self.file.write_index.load(Ordering::SeqCst) as u64;
            other.write_line_at(other_last_pos, &SegmentLine::Jump(Some(write_index))).map_err(CacheError::WriteError)?;

            {
                let mut other_seek_table = other.seek_table.write().unwrap();
                let mut seek_table = self.seek_table.write().unwrap();

                other_seek_table.pop();
                let last = seek_table.pop();
                let mut correct = false;
                for other_point in other_seek_table.iter() {
                    if let Some(last_point) = &last && !correct {
                        if last_point.stream == other_point.stream && last_point.pts < other_point.pts {
                            correct = true;
                        }
                    }
                    seek_table.push(other_point.clone());
                }
                seek_table.sort_by(|a, b| a.pts.total_cmp(&b.pts));
            }

            self.write_packet(packet.clone()).map_err(CacheError::WriteError)?;

            self.size.fetch_add(other.size.load(Ordering::Relaxed) as u64, Ordering::Relaxed);
            self.set_sync()?;
        }
        Ok(())
    }

    fn packet_pts(&self, packet: &Packet) -> Option<f64> {
        let pts = packet.pts();
        if pts == AV_NOPTS_VALUE {
            return None
        }
        let stream = &self.stream_meta[packet.stream_index() as usize];
        Some((pts as f64) * stream.timebase)
    }

    fn handle_sync(&mut self, input: &mut Input) -> Result<(), CacheError> {
        let stage = match self.sync_stage.take() {
            Some(stage) => stage,
            None => return Ok(()),
        };
        match stage {
            SegmentSyncStage::SyncSeek(pts, packet) => {
                input.seek(f64::MIN, pts, None).map_err(CacheError::SourceReadError)?;
                loop {
                    let read = input.read_packet().map_err(CacheError::SourceReadError)?;
                    if read == packet {
                        break
                    }
                }
            }
            SegmentSyncStage::InitSeek(pts) => {
                input.seek(f64::MIN, pts, None).map_err(CacheError::SourceReadError)?;
            }
            SegmentSyncStage::SyncForward(..) => {}
        }
        Ok(())
    }

    fn set_sync(&mut self) -> Result<(), CacheError> {
        let last_pos = match self.last_key {
            Some(index) => index as usize,
            None => return Ok(())
        };
        let (last_packet, _) = match self.read_packet_at(last_pos).map_err(CacheError::ReadError)? {
            Some(packet) => packet,
            None => return Ok(()),
        };
        self.write_line_at(last_pos, &SegmentLine::Jump(Some(self.file.write_index.load(Ordering::SeqCst) as u64)))
            .map_err(CacheError::WriteError)?;
        self.write_packet(last_packet.clone()).map_err(CacheError::WriteError)?;
        self.sync_stage = Some(SegmentSyncStage::SyncSeek(self.packet_pts(&last_packet).unwrap(), last_packet));
        Ok(())
    }

    fn cached(&self) -> f64 {
        let begin = self.begin.load(Ordering::SeqCst);
        let end = self.end.load(Ordering::SeqCst);
        if begin.is_nan() || end.is_nan() {
            0.0
        } else {
            end - begin
        }
    }
}

#[derive(Debug)]
pub enum CacheError {
    SourceReadError(ffmpeg::error::Error),
    WriteError(std::io::Error),
    ReadError(std::io::Error),
    SyncError,
    Eof
}

#[derive(Clone)]
pub struct SegmentView {
    begin: Arc<AtomicF64>,
    end: Arc<AtomicF64>,
    size: Arc<AtomicU64>,
    sealed: Arc<AtomicBool>,
    seek_table: Arc<RwLock<Vec<SeekPoint>>>,
}

impl SegmentView {

    fn empty() -> SegmentView {
        SegmentView {
            begin: Arc::new(AtomicF64::new(f64::NAN)),
            end: Arc::new(AtomicF64::new(f64::NAN)),
            size: Arc::new(AtomicU64::new(0)),
            sealed: Arc::new(AtomicBool::new(false)),
            seek_table: Arc::new(RwLock::new(Vec::new())),
        }
    }

    fn from(segment: &Segment) -> Self {
        Self {
            begin: segment.begin.clone(),
            end: segment.end.clone(),
            size: segment.size.clone(),
            sealed: segment.sealed.clone(),
            seek_table: segment.seek_table.clone(),
        }
    }

    pub fn range(&self) -> Range<f64> {
        self.begin.load(Ordering::SeqCst)..self.end.load(Ordering::SeqCst)
    }

    pub fn size(&self) -> u64 {
        self.size.load(Ordering::SeqCst)
    }

    fn seek(&self, pts: f64) -> Option<SeekPoint> {
        if !self.contains(pts) { return None }
        let table = self.seek_table.read().unwrap();
        let res = table.binary_search_by(|p| p.pts.total_cmp(&pts)).unwrap_or_else(|e| e);
        table.get(res.max(1) - 1).cloned()
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::SeqCst)
    }

    pub fn contains(&self, pts: f64) -> bool {
        let begin = self.begin.load(Ordering::SeqCst);
        let end = self.end.load(Ordering::SeqCst);
        !begin.is_nan() && !end.is_nan() && pts >= begin && pts <= end
    }

}

pub struct SegmentReader {
    seek_table: Arc<RwLock<Vec<SeekPoint>>>,
    begin: Arc<AtomicF64>,
    end: Arc<AtomicF64>,
    sealed: Arc<AtomicBool>,
    streams: Vec<StreamMetaData>,
    current_pts: Option<f64>,
    buffer: ByteBuffer
}

impl SegmentReader {

    pub fn packet_pts(&self, packet: &Packet) -> Option<f64> {
        let pts = packet.pts();
        let stream = &self.streams[packet.stream_index() as usize];

        Some(pts).take_if(|pts| *pts != AV_NOPTS_VALUE)
            .map(|pts| pts as f64)
            .map(|pts| stream.start_time * pts)
    }

    fn from_segment(segment: &Segment) -> Self {
        Self {
            seek_table: segment.seek_table.clone(),
            streams: segment.stream_meta.clone(),
            begin: segment.begin.clone(),
            end: segment.end.clone(),
            sealed: segment.sealed.clone(),
            current_pts: None,
            buffer: ByteBuffer::new(1024)
        }
    }

    pub fn cached(&self) -> Option<f64> {
        let begin = self.begin.load(Ordering::SeqCst);
        let end = self.end.load(Ordering::SeqCst);
        Some(end - self.current_pts.unwrap_or(begin))
            .take_if(|d| !d.is_nan())
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::SeqCst)
    }

    pub fn contains(&self, pts: f64) -> bool {
        let begin = self.begin.load(Ordering::SeqCst);
        let end = self.end.load(Ordering::SeqCst);
        !begin.is_nan() && !end.is_nan() && pts >= begin && pts <= end
    }
}

pub struct Cache {
    file: CacheFile,
    input: Input,
    streams: Vec<StreamMetaData>,
    segments: Vec<Segment>,

    seek_requests: Receiver<f64>,
    seek_sender: Sender<f64>,

    new_segment_listeners: Vec<Sender<u64>>,

    segment_views: Arc<RwLock<Vec<SegmentView>>>,

    current: Segment,
    current_segment_reader: Arc<RwLock<SegmentView>>,

    read_error: bool,
    check_pass: bool,
    pub closed: bool,
}

impl Cache {

    pub fn new<T: AsRef<Path>>(path: T, input: Input) -> Self {
        let file = CacheFile::new(path);
        let meta = input.streams.iter().map(Stream::metadata).collect::<Vec<_>>();
        let segment = Segment::new(file.clone(), meta.clone());
        let segment_views = Arc::new(RwLock::new(vec![SegmentView::from(&segment)]));
        let (sender, receiver) = mpsc::channel();
        let mut cache = Self {
            segments: Vec::new(),
            segment_views,
            current_segment_reader: Arc::new(RwLock::new(SegmentView::from(&segment))),
            current: segment,
            streams: meta,
            seek_requests: receiver,
            seek_sender: sender,
            new_segment_listeners: Vec::new(),
            file,
            input,
            read_error: false,
            check_pass: true,
            closed: false,
        };
        cache.init();
        cache
    }

    fn init(&mut self) {
        for stream in &self.input.streams {
            self.current.write_line(&SegmentLine::Stream(stream.clone())).unwrap();
        }
    }

    fn new_segment(&mut self) -> Segment {
        let new = Segment::new(self.file.clone(), self.streams.clone());
        let index = self.file.write_index.load(Ordering::SeqCst);
        self.new_segment_listeners.retain(|listener| {
            listener.send(index as u64).ok().is_some()
        });
        new
    }

    pub fn views(&self) -> Arc<RwLock<Vec<SegmentView>>> {
        self.segment_views.clone()
    }

    pub fn has_source_error(&self) -> bool {
        self.input.read_error
    }

    pub fn has_error(&self) -> bool {
        self.read_error
    }

    fn update_segment_views(&mut self) {
        let mut views = self.segment_views.write().unwrap();
        views.clear();
        for segment in self.segments.iter() {
            views.push(SegmentView::from(&segment));
        }
        views.push(SegmentView::from(&self.current));
    }

    fn check_overlap_and_merge(&mut self) -> Result<(), CacheError> {
        let overlapping = self.segments.iter()
            .enumerate()
            .find(|(index, other)| {
                self.current.overlaps_with(other)
            }).map(|(index, _)| index);
        if let Some(other) = overlapping.map(|index| self.segments.remove(index)) {
            self.update_segment_views();
            self.current.try_merge(other)?;
        }
        Ok(())
    }

    pub fn process_requests(&mut self) -> Result<(), CacheError> {
        if let Some(pts) = self.seek_requests.try_iter().last() {
            self.seek(pts)?;
        }
        Ok(())
    }

    pub fn do_pass(&mut self) -> Result<(), CacheError> {
        self.check_pass = false;
        self.current.handle_sync(&mut self.input)?;
        let packet = match self.input.read_packet() {
            Ok(packet) => packet,
            Err(err) => {
                if err.is_eof() {
                    self.current.set_sealed().map_err(CacheError::WriteError)?;
                }
                return Ok(());
            },
        };
        let key = packet.is_key();
        self.current.write_packet(packet).map_err(CacheError::WriteError)?;
        if key {
            self.check_overlap_and_merge()?;
        }
        Ok(())
    }

    pub fn write_packet(&mut self, packet: Packet) -> Result<(), CacheError> {
        let key = packet.is_key();
        self.current.write_packet(packet).map_err(CacheError::WriteError)?;
        if key {
            self.check_overlap_and_merge()?;
        }
        Ok(())
    }

    pub fn seek(&mut self, target: f64) -> Result<(), CacheError> {
        {
            if self.current.seek(target) {
                self.read_error = false;
                return Ok(())
            }
        }

        let index = self.segments
            .iter_mut()
            .enumerate()
            .find_map(|(index, s)| if s.seek(target) { Some(index) } else { None });
        if let Some(mut other) = index.map(|index| self.segments.remove(index)) {
            other.continue_from_end(&mut self.current)?;
            let last = replace(&mut self.current, other);
            self.segments.push(last);
        } else {
            self.current.end().map_err(CacheError::WriteError)?;
            let segment = self.new_segment();
            let last = replace(&mut self.current, segment);
            self.segments.push(last);
            self.input.seek(f64::MIN, target, None).map_err(CacheError::SourceReadError)?;
        }
        self.update_segment_views();
        self.update_reader();

        self.read_error = false;
        self.check_pass = true;

        Ok(())
    }

    fn update_reader(&mut self) {
        let mut reader = self.current_segment_reader.write().unwrap();
        *reader = SegmentView::from(&self.current);
    }

    pub fn is_current_sealed(&self) -> bool {
        self.current.is_sealed()
    }

    pub fn reader(&mut self) -> CacheReader {
        let (sender, receiver) = mpsc::channel();
        self.new_segment_listeners.push(sender);
        CacheReader {
            current_segment_reader: self.current_segment_reader.clone(),
            sender: Some(self.seek_sender.clone()),
            new_segment_listener: Some(receiver),
            await_new: false,
            segment_views: self.segment_views.clone(),
            buffer: ByteBuffer::new(1024),
            streams: self.streams.clone(),
            file: self.file.clone(),
            current_pts: None,
            managed: true,
            serial: 0
        }
    }

    pub fn cached_duration(&self) -> Option<f64> {
        let end = self.current.end.load(Ordering::SeqCst);
        let begin = self.current.begin.load(Ordering::SeqCst);
        let duration = end - self.current.current_pts.unwrap_or(begin);
        Some(duration).take_if(|d| !d.is_nan())
    }

}

pub struct CacheReader {
    current_segment_reader: Arc<RwLock<SegmentView>>,
    pub(super) segment_views: Arc<RwLock<Vec<SegmentView>>>,
    new_segment_listener: Option<Receiver<u64>>,
    await_new: bool,
    pub(super) streams: Vec<StreamMetaData>,
    buffer: ByteBuffer,
    file: CacheFile,
    sender: Option<Sender<f64>>,
    current_pts: Option<f64>,
    managed: bool,
    pub serial: u32
}

impl CacheReader {

    pub(crate) fn load<P: AsRef<Path>>(path: P) -> Result<(CacheReader, Vec<Stream>), CacheError> {
        let mut file = CacheFile::open(path).map_err(CacheError::ReadError)?;

        let mut buffer = ByteBuffer::new(1024);
        let mut streams = Vec::new();
        let mut segments = Vec::<SegmentView>::new();
        let mut current = SegmentView::empty();
        let mut last_offset = Vec::<u64>::new();
        let mut preferred_stream = None;
        loop {
            let before_index = file.read_index;
            if before_index >= file.write_index.load(Ordering::SeqCst) { break }
            buffer.clear();
            let fail = file.read_packet(&mut buffer).is_err();
            if fail {
                break
            }
            let line = match buffer.read_ser::<SegmentLineRead>() {
                Some(line) => line,
                None => break
            };
            match line {
                SegmentLineRead::Seal => {
                    let current = replace(&mut current, SegmentView::empty());
                    current.sealed.store(true, Ordering::SeqCst);
                    segments.push(current);

                    if let Some(last) = last_offset.pop() {
                        file.read_index = last as usize;
                    } else {
                        break
                    }
                }
                SegmentLineRead::Stream(stream) => {
                    if stream.stream_type == StreamType::Video {
                        preferred_stream = Some(stream.index);
                    }
                    streams.push(stream)
                },
                SegmentLineRead::Jump(offset) => {
                    if let Some(offset) = offset {
                        last_offset.push(file.read_index as u64);
                        file.read_index = offset as usize;
                    } else {
                        let current = replace(&mut current, SegmentView::empty());
                        segments.push(current);
                        if let Some(last) = last_offset.pop() {
                            file.read_index = last as usize;
                        } else {
                            break
                        }
                    }
                }
                SegmentLineRead::Packet(pts, stream_index, flags) => {
                    let stream = &streams[stream_index as usize];
                    let pts = Some(pts)
                        .take_if(|_| preferred_stream.map(|index| index == stream_index).unwrap_or(true))
                        .take_if(|pts| *pts != AV_NOPTS_VALUE && (flags & AV_PKT_FLAG_KEY > 0))
                        .map(|pts| pts as f64)
                        .map(|pts| pts * stream.timebase)
                        .take_if(|pts| {
                            if preferred_stream.is_some() {
                                true
                            } else {
                                current.seek_table.read().unwrap().last()
                                    .take_if(|point| (*pts - point.pts) >= 0.5)
                                    .is_some()
                            }
                        });
                    if let Some(pts) = pts {
                        let begin = current.begin.load(Ordering::SeqCst);
                        let end = current.end.load(Ordering::SeqCst);
                        if begin.is_nan() {
                            current.begin.store(pts, Ordering::SeqCst);
                        }
                        if end.is_nan() || pts > end {
                            current.end.store(pts, Ordering::SeqCst);
                        }
                        current.seek_table.write().unwrap().push(SeekPoint {
                            pts,
                            stream: stream_index,
                            offset: before_index
                        })
                    }
                }
            }
        }
        segments.push(current);

        file.read_index = 0;
        buffer.clear();
        segments.retain(|s| !s.begin.load(Ordering::SeqCst).is_nan() && !s.end.load(Ordering::SeqCst).is_nan());
        segments.sort_by(|a, b| a.begin.load(Ordering::SeqCst).total_cmp(&b.begin.load(Ordering::SeqCst)));
        let first = segments.first().cloned().ok_or_else(|| CacheError::Eof)?;

        Ok((CacheReader {
            current_segment_reader: Arc::new(RwLock::new(first)),
            segment_views: Arc::new(RwLock::new(segments)),
            new_segment_listener: None,
            await_new: false,
            streams: streams.iter().map(Stream::metadata).collect(),
            buffer,
            file,
            sender: None,
            current_pts: None,
            managed: false,
            serial: 0
        }, streams))
    }

    pub fn duration(&self) -> f64 {
        self.streams.iter().map(|s| s.duration)
            .max_by(|a, b| a.total_cmp(b)).unwrap_or(0.0)
    }

    pub fn start_time(&self) -> f64 {
        self.streams.iter().map(|s| s.start_time)
            .min_by(|a, b| a.total_cmp(b)).unwrap_or(0.0)
    }

    fn segment(&'_ self) -> RwLockReadGuard<'_, SegmentView> {
        self.current_segment_reader.read().unwrap()
    }

    pub fn cached_duration(&mut self) -> f64 {
        if self.await_new && let Some(listener) = self.new_segment_listener.as_ref() {
            if let Some(offset) = listener.try_recv().ok() {
                self.file.read_index = offset as usize;
                self.await_new = false;
            }
            return 0.0;
        }
        let range = self.segment().range();
        if range.start.is_nan() || range.end.is_nan() {
            return 0.0;
        }
        if self.current_pts.is_none() {
            return f64::MAX;
        }
        range.end - self.current_pts.unwrap()
    }

    fn packet_pts(&self, packet: &Packet) -> Option<f64> {
        let pts = packet.pts();
        Some(pts).take_if(|p| *p != AV_NOPTS_VALUE)
            .map(|pts| pts as f64)
            .map(|pts| {
                let stream = &self.streams[packet.stream_index() as usize];
                pts * stream.timebase
            })
    }

    pub fn read_packet(&mut self) -> Result<Packet, CacheError> {
        loop {
            if self.file.read_index >= self.file.write_index.load(Ordering::SeqCst) {
                return Err(CacheError::Eof)
            }
            self.buffer.clear();
            self.file.read_packet(&mut self.buffer).map_err(CacheError::ReadError)?;
            let line: SegmentLine = self.buffer.read_ser()
                .ok_or_else(|| CacheError::ReadError(std::io::Error::new(ErrorKind::Other, "Failed to read segment line")))?;
            match line {
                SegmentLine::Jump(jump) => {
                    if let Some(jump) = &jump {
                        self.file.read_index = *jump as usize;
                        continue;
                    } else {
                        return Err(CacheError::Eof)
                    }
                }
                SegmentLine::Packet(mut packet) => {
                    self.current_pts = self.packet_pts(&packet).or(self.current_pts);
                    packet.serial = self.serial;
                    return Ok(packet)
                },
                SegmentLine::Stream(_) => {}
                SegmentLine::Seal => return Err(CacheError::Eof),
            }
        }
    }

    pub fn is_sealed(&self) -> bool {
        self.segment().is_sealed()
    }

    pub fn seek(&mut self, target: f64) {
        self.serial += 1;
        self.current_pts = None;
        self.await_new = false;

        {
            let current = self.segment();
            if let Some(point) = current.seek(target) {
                drop(current);
                self.file.read_index = point.offset;
                return;
            }
        }

        let offset = self.segment_views.read()
            .unwrap().iter()
            .find_map(|view| view.seek(target).map(|p| (p, view.clone())));

        if let Some((point, view)) = offset {
            self.file.read_index = point.offset;
            if !self.managed {
                *self.current_segment_reader.write().unwrap() = view;
            }
        } else if self.managed {
            self.await_new = true;
        } else {
            return;
        }

        if let Some(sender) = self.sender.as_ref() {
            sender.send(target).unwrap();
        }
    }

    fn reset_buffers(&mut self) {
        self.buffer = ByteBuffer::new(1024);
        self.file.buffer = ByteBuffer::new(1024);
    }
}

struct CacheWorkerJob {
    cache: Cache,
    notifier: InputWorkerNotifier
}

enum CacheWorkerMessage {
    End,
    Update,
    Job(CacheWorkerJob)
}

pub struct CacheWorker {
    thread: JoinHandle<()>,
    sender: Sender<CacheWorkerMessage>,
}

pub struct CacheWorkerContext {
    jobs: Vec<CacheWorkerJob>,
    receiver: Receiver<CacheWorkerMessage>,
}

impl CacheWorkerContext {

    fn drain_messages(&mut self) -> bool {
        let mut update = false;
        while let Some(msg) = self.receiver.try_recv().ok() {
            if self.handle_msg(msg) {
                update = true;
            }
        }
        update
    }

    fn handle_msg(&mut self, msg: CacheWorkerMessage) -> bool {
        match msg {
            CacheWorkerMessage::Update => true,
            CacheWorkerMessage::Job(job) => {
                self.jobs.push(job);
                true
            }
            CacheWorkerMessage::End => {
                unimplemented!()
            }
        }
    }

    fn await_message(&mut self) {
        let msg = self.receiver.recv().unwrap();
        self.handle_msg(msg);
    }

    fn clear_dead_jobs(&mut self) {
        self.jobs.retain(|job| !job.cache.closed)
    }

    fn do_pass(&mut self) -> Result<bool, CacheError> {
        let mut empty = true;
        self.clear_dead_jobs();
        for job in self.jobs.iter_mut() {
            job.cache.process_requests()?;
            let error_or_ended = job.cache.has_error() || job.cache.is_current_sealed() || job.cache.has_source_error();
            if error_or_ended && !job.cache.check_pass {
                continue;
            }
            job.cache.do_pass()?;
            job.notifier.notify();
            empty = false;
        }
        Ok(empty)
    }

    fn run(&mut self) {
        loop {
            loop {
                let update = self.drain_messages();
                let empty = self.do_pass().unwrap_or(false);
                if !update && empty {
                    break;
                }
            }
            self.await_message();
        }
    }

}

pub struct CacheWorkerNotifier(Sender<CacheWorkerMessage>);

impl CacheWorkerNotifier {
    pub fn notify(&self) {
        self.0.send(CacheWorkerMessage::Update).unwrap();
    }
}

impl CacheWorker {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut context = CacheWorkerContext { jobs: Vec::new(), receiver };
        let thread = std::thread::spawn(move || {
            context.run();
        });

        Self { thread, sender }
    }

    pub fn push(&mut self, mut cache: Cache, notifier: InputWorkerNotifier) -> (CacheReader, CacheWorkerNotifier) {
        let reader = cache.reader();
        self.sender.send(CacheWorkerMessage::Job(CacheWorkerJob {
            cache,
            notifier,
        })).unwrap();
        (reader, CacheWorkerNotifier(self.sender.clone()))
    }

}