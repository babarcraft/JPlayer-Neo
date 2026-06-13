use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::ops::Range;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
use ffmpeg_sys_next::{register_t, AV_NOPTS_VALUE, AV_PKT_FLAG_KEY};
use crate::ffmpeg;
use crate::ffmpeg::input::{Input, Stream, StreamMetaData};
use crate::ffmpeg::packet::{ByteBuffer, Packet, Serializable};
use crate::player::clock::AtomicF64;

enum SegmentLine {
    Packet(Packet),
    Jump(Option<u64>)
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
            _ => None
        }
    }
}

#[derive(Clone)]
pub struct CacheFile {
    file: Arc<File>,
    buffer: ByteBuffer,
    read_index: usize,
    write_index: usize,
}

impl CacheFile {
    pub fn new<T: AsRef<Path>>(file: T) -> Self {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(file)
            .unwrap();
        Self {
            file: Arc::new(file),
            buffer: ByteBuffer::new(1024),
            read_index: 0,
            write_index: 0
        }
    }

    pub fn write_packet(&mut self, buffer: &ByteBuffer) -> Result<(), std::io::Error> {
        let index = self.write_index;
        self.buffer.clear();
        self.buffer.write_ser(&(buffer.len() as u32));
        self.buffer.write_ser(&buffer.crc_32());
        self.buffer.write(buffer.internal());
        let len = self.file.write_at(self.buffer.internal(), index as u64)?;
        self.write_index += len;
        Ok(())
    }

    pub fn read_packet(&mut self, buffer: &mut ByteBuffer) -> Result<(), std::io::Error> {
        self.buffer.clear();
        {
            let dest = self.buffer.internal_mut(size_of::<u32>() * 2);
            self.read_index += self.file.read_at(dest, self.read_index as u64)?;
        }
        let len = self.buffer.read_ser::<u32>().ok_or_else(||std::io::Error::new(ErrorKind::Other, "Could not read packet size."))?;
        let crc = self.buffer.read_ser::<u32>().ok_or_else(||std::io::Error::new(ErrorKind::Other, "Could not read packet crc."))?;

        {
            let dest = buffer.internal_mut(len as usize);
            self.read_index += self.file.read_at(dest, self.read_index as u64)?;
        }
        if crc != buffer.crc_32() {
            Err(std::io::Error::new(ErrorKind::Other, "File corrupted!"))
        } else {
            Ok(())
        }
    }

}

pub struct AtomicOption<T: Sized + Clone> {
    ptr: AtomicPtr<T>
}

impl<T: Sized + Clone> AtomicOption<T> {
    pub fn new(value: Option<T>) -> Self {
        Self {
            ptr: AtomicPtr::new(value.map(|v| Box::into_raw(Box::new(v))).unwrap_or(null_mut()))
        }
    }

    pub fn load(&self, order: Ordering) -> Option<T> {
        let ptr = self.ptr.load(order);
        if ptr.is_null() {
            None
        } else {
            let opt = unsafe { &mut *ptr };
            Some(opt.clone())
        }
    }

    pub fn store(&self, val: T, order: Ordering) {
        let data = Box::new(val);
        unsafe {
            let last = self.ptr.swap(Box::into_raw(data), order);
            if last != null_mut() {
                drop(Box::from_raw(last));
            }
        }
    }

    pub fn take(&self) -> Option<T> {
        let ptr = self.ptr.swap(null_mut(), Ordering::SeqCst);
        if ptr.is_null() {
            None
        } else {
            let data = unsafe { Box::from_raw(ptr) };
            Some(*data)
        }
    }
}

impl<T: Sized + Clone> Drop for AtomicOption<T> {
    fn drop(&mut self) {
        drop(self.take());
    }
}

pub struct Segment {
    file: CacheFile,
    buffer: ByteBuffer,
    range: Range<usize>,
    begin: AtomicF64,
    end: AtomicF64,
    first_key: Option<u64>,
    last_key: Option<u64>,
    stream_meta: Vec<StreamMetaData>,
    seek_table: Vec<(f64, u64)>,
    empty_jump: Option<u64>,
}

impl Segment {

    fn new(mut file: CacheFile, range: Range<usize>, stream_meta_data: Vec<StreamMetaData>) -> Self {
        file.write_index = range.start;
        file.read_index = range.start;
        Self {
            file,
            buffer: ByteBuffer::new(1024),
            range,
            begin: AtomicF64::new(f64::NAN),
            end: AtomicF64::new(f64::NAN),
            stream_meta: stream_meta_data,
            seek_table: Vec::new(),
            empty_jump: None,
            last_key: None,
            first_key: None,
        }
    }

    fn continue_from_end(&mut self, other: &mut Segment) -> Result<(), std::io::Error> {
        other.end()?;
        if let Some(jump_index) = self.empty_jump.take() {
            self.file.write_index = jump_index as usize;
            let end = other.range.end;
            self.write_line(&SegmentLine::Jump(Some(end as u64)))?;
            self.range.end = end;
            self.file.write_index = end;
        }

        Ok(())
    }

    fn write_line(&mut self, line: &SegmentLine) -> Result<(), std::io::Error> {
        self.buffer.clear();
        self.buffer.write_ser(line);
        self.file.write_packet(&self.buffer)?;
        self.range.end = self.file.write_index;
        Ok(())
    }

    fn read_line(&mut self) -> Result<SegmentLine, std::io::Error> {
        self.buffer.clear();
        self.file.read_packet(&mut self.buffer)?;
        self.buffer.read_ser::<SegmentLine>().ok_or_else(||std::io::Error::new(ErrorKind::Other, "Could not read line."))
    }

    fn update(&mut self, packet: &Packet) {
        let pts = packet.pts();
        let stream = &self.stream_meta[packet.stream_index() as usize];
        if pts != AV_NOPTS_VALUE && packet.is_key() {
            let pts = (pts as f64) * stream.timebase;
            let begin = self.begin.load(Ordering::SeqCst);
            let end = self.end.load(Ordering::SeqCst);
            if pts < begin || begin.is_nan() {
                self.begin.store(pts, Ordering::SeqCst);
            }
            if end < pts || end.is_nan() {
                self.end.store(pts, Ordering::SeqCst);
            }
            let push = self.seek_table.last()
                .take_if(|(last_pts, _)| pts - *last_pts < 0.5)
                .is_none();
            let index = self.file.write_index as u64;
            self.first_key.get_or_insert(index);
            self.last_key = Some(index);
            if push {
                self.seek_table.push((pts, index));
            }
        }
    }

    fn seek(&mut self, target: f64) -> bool {
        let begin = self.begin.load(Ordering::SeqCst);
        let end = self.end.load(Ordering::SeqCst);
        if target < begin || target > end {
            return false;
        }

        let find = self.seek_table.iter().enumerate()
            .find(|(_, (pts, pos))| *pts > target)
            .filter(|(idx, _)| *idx > 0)
            .map(|(idx, _)| self.seek_table[idx].1)
            .unwrap_or(self.range.start as u64);
        self.file.read_index = find as usize;
        true
    }

    fn read_packet(&mut self) -> Result<Option<Packet>, std::io::Error> {
        if self.file.read_index >= self.file.write_index {
            return Ok(None);
        }
        let line = self.read_line()?;
        match line {
            SegmentLine::Packet(packet) => {
                Ok(Some(packet))
            }
            SegmentLine::Jump(jump) => {
                if let Some(jump) = jump {
                    self.file.read_index = jump as usize;
                    self.read_packet()
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn write_packet(&mut self, packet: Packet) -> Result<(), std::io::Error> {
        self.update(&packet);
        self.write_line(&SegmentLine::Packet(packet))?;
        Ok(())
    }

    fn end(&mut self) -> Result<(), std::io::Error> {
        self.empty_jump = Some(self.file.write_index as u64);
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

    fn try_merge(&mut self, mut other: Segment) -> Result<(), std::io::Error> {
        let last = self.file.read_index;

        self.file.read_index = match self.last_key {
            Some(index) => index as usize,
            None => return Ok(())
        };
        let last_packet_pos = self.file.read_index;
        let last_packet = match self.read_packet()? {
            Some(packet) => packet,
            None => return Ok(()),
        };
        other.file.read_index = match other.first_key {
            Some(index) => index as usize,
            None => return Ok(())
        };
        let mut found = false;
        while let Some(packet) = other.read_packet()? {
            if packet == last_packet {
                found = true;
                break
            }
        }

        if found {
            let last = self.file.write_index;
            self.file.write_index = last_packet_pos;
            self.write_line(&SegmentLine::Jump(Some(last as u64)))?;
            self.file.write_index = last;
            self.write_packet(last_packet)?;
            while let Some(packet) = other.read_packet()? {
                self.write_packet(packet)?;
            }
        }

        self.file.read_index = last;
        Ok(())
    }

    fn sync_input(&mut self, input: &mut Input) -> Result<(), CacheError> {
        let last = self.file.read_index;
        self.file.read_index = match self.last_key {
            Some(index) => index as usize,
            None => return Ok(())
        };
        let last_packet_pos = self.file.read_index;
        let last_packet = match self.read_packet().map_err(CacheError::ReadError)? {
            Some(packet) => packet,
            None => return Ok(()),
        };
        let pts = last_packet.pts();
        let stream = &self.stream_meta[last_packet.stream_index() as usize];
        let pts = (pts as f64) * stream.timebase;
        input.seek(0.0, pts, None).map_err(CacheError::SourceReadError)?;
        loop {
            let packet = input.read_packet().map_err(CacheError::SourceReadError)?;
            if packet == last_packet {
                break
            }
            let this_pts = packet.pts();
            let stream = &self.stream_meta[packet.stream_index() as usize];
            let this_pts = (this_pts as f64) * stream.timebase;
            if this_pts > pts {
                // return Err(CacheError::SyncError)
            }
        }
        {
            let last = self.file.write_index;
            self.file.write_index = last_packet_pos;
            self.write_line(&SegmentLine::Jump(Some(last as u64))).map_err(CacheError::WriteError)?;
            self.file.write_index = last;
            self.write_packet(last_packet).map_err(CacheError::WriteError)?;
        }
        self.file.read_index = last;
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

pub struct Cache {
    file: CacheFile,
    pub input: Input,
    stream_meta: Vec<StreamMetaData>,
    segments: Vec<Option<Segment>>,
    current_segment: usize,
    read_error: bool,
    pub serial: u32,

    begin: Option<Instant>,
}

impl Cache {

    pub fn new<T: AsRef<Path>>(path: T, input: Input) -> Self {
        let mut segments = Vec::new();
        let file = CacheFile::new(path);
        let meta = input.streams.iter().map(Stream::metadata).collect::<Vec<_>>();
        let segment = Segment::new(file.clone(), 0..0, meta.clone());
        segments.push(Some(segment));
        Self {
            segments,
            current_segment: 0,
            stream_meta: meta,
            file,
            input,
            read_error: false,
            serial: 0,
            begin: None,
        }
    }

    fn new_segment(&mut self) -> usize {
        let current = self.segments[self.current_segment].as_ref().unwrap();
        let segment = Segment::new(self.file.clone(), current.range.end..current.range.end, self.stream_meta.clone());
        let slot = self.segments.iter_mut()
            .enumerate().find(|(_, s)| s.is_none()).map(|(i, _)| i);
        if let Some(slot) = slot {
            self.segments[slot] = Some(segment);
            slot
        } else {
            self.segments.push(Some(segment));
            self.segments.len() - 1
        }
    }

    pub fn has_error(&self) -> bool {
        self.input.read_error && self.read_error
    }

    fn check_overlap_and_merge(&mut self) -> Result<(), CacheError> {
        let overlapping = self.segments.iter()
            .enumerate()
            .filter_map(|(index, s)| Some(index).zip(s.as_ref()))
            .find(|(index, other)| {
                let current = self.segments[self.current_segment].as_ref().unwrap();
                current.overlaps_with(other) && *index != self.current_segment
            }).map(|(index, _)| index);
        if let Some(other) = overlapping.and_then(|index| self.segments[index].take()) {
            let current = self.segments[self.current_segment].as_mut().unwrap();
            current.try_merge(other).map_err(CacheError::WriteError)?;
            current.sync_input(&mut self.input)?;
        }
        Ok(())
    }

    pub fn write_next(&mut self) -> Result<(), CacheError> {
        let inst = self.begin.get_or_insert_with(Instant::now);
        let current = self.segments[self.current_segment].as_ref().unwrap();
        let speed = current.cached() / inst.elapsed().as_secs_f64();
        if inst.elapsed().as_secs() % 3 == 0 {
            println!("==========");
            for (index, seg) in self.segments.iter().filter_map(|s| s.as_ref()).enumerate() {
                println!("Segment {index} => {} - {}", seg.begin.load(Ordering::SeqCst), seg.end.load(Ordering::SeqCst))
            }
        }
        let current = self.segments[self.current_segment].as_mut().unwrap();
        let packet = self.input.read_packet().map_err(CacheError::SourceReadError)?;
        let is_key = packet.is_key();
        current.write_packet(packet).map_err(CacheError::WriteError)?;

        if is_key {
            self.check_overlap_and_merge()?;
        }
        Ok(())
    }

    pub fn read_packet(&mut self) -> Result<Packet, CacheError> {
        let current = self.segments[self.current_segment].as_mut().unwrap();
        let packet = current.read_packet().map_err(CacheError::ReadError);
        match packet {
            Ok(packet) => match packet {
                Some(mut packet) => {
                    packet.serial = self.serial;
                    Ok(packet)
                },
                None => {
                    self.read_error = true;
                    Err(CacheError::Eof)
                }
            },
            Err(err) => Err(err)
        }
    }

    pub fn seek(&mut self, target: f64) -> Result<(), CacheError> {
        self.serial += 1;
        self.begin = Some(Instant::now());
        {
            let current = self.segments[self.current_segment].as_mut().unwrap();
            if current.seek(target) {
                self.read_error = false;
                return Ok(())
            }
        }

        let index = self.segments
            .iter_mut()
            .enumerate()
            .filter_map(|(index, s)| Some(index).zip(s.as_mut()))
            .find_map(|(index, s)| if s.seek(target) { Some(index) } else { None });
        if let Some(index) = index {
            let mut other = self.segments[index].take().unwrap();
            let current = self.segments[self.current_segment].as_mut().unwrap();
            let res = other.continue_from_end(current)
                    .map_err(CacheError::WriteError)
                .and(other.sync_input(&mut self.input));
            self.segments[index] = Some(other);
            res?;
            self.current_segment = index;
        } else {
            self.input.seek(0.0, target, None).map_err(CacheError::SourceReadError)?;
            let current = self.segments[self.current_segment].as_mut().unwrap();
            current.end().map_err(CacheError::WriteError)?;
            self.current_segment = self.new_segment();
        }

        self.read_error = false;

        Ok(())
    }

}