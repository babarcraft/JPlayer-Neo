use crate::ffmpeg::current_time;
use crate::ffmpeg::decode::{AudioConverter, Decoder, DecoderResult};
use crate::ffmpeg::frame::{AudioFrame, Frame};
use crate::ffmpeg::input::{Stream, StreamType};
use crate::player::clock::{AtomicF64, AtomicInstant, Clock};
use crate::player::decoder::FrameConsumer::{AudioConsumer, VideoConsumer};
use crate::player::input::{InputJobHandle, PacketQueue, PacketQueueView};
use crate::player::surface::{FrameQueue, FrameQueueView};
use ffmpeg_sys_next::AVSampleFormat::AV_SAMPLE_FMT_S16;
use std::mem::transmute;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct AudioRingBuffer {
    buffer: Vec<i16>,
    write_index: usize,
    read_index: usize,

    view: AudioRingView,

    pts: Arc<AtomicF64>,
    samples_read: Arc<AtomicUsize>,
    last_read: Arc<AtomicInstant>,
    seek: Arc<AtomicF64>,

    pub latency: Option<f64>,

    pub sample_rate: u32,
    pub channels: u16,

    pub skips: usize,
}

#[derive(Clone)]
pub struct AudioRingView {
    size: Arc<AtomicUsize>,
    serial: Arc<AtomicU32>,
    closed: Arc<AtomicBool>,
    capacity: usize,
}

impl AudioRingView {
    pub fn size(&self) -> usize {
        self.size.load(Ordering::SeqCst)
    }

    pub fn remaining(&self) -> usize {
        self.capacity - self.size()
    }

    pub fn serial(&self) -> u32 {
        self.serial.load(Ordering::SeqCst)
    }

    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl AudioRingBuffer {
    pub fn new(seconds: f32, sample_rate: u32, channels: u16) -> AudioRingBuffer {
        let size = (seconds * (sample_rate as f32) * (channels as f32)).round() as usize;
        AudioRingBuffer {
            sample_rate,
            channels,
            buffer: vec![0i16; size],
            write_index: 0,
            read_index: 0,

            view: AudioRingView {
                serial: Arc::new(AtomicU32::new(0)),
                size: Arc::new(AtomicUsize::new(0)),
                closed: Arc::new(AtomicBool::new(false)),
                capacity: size,
            },

            samples_read: Arc::new(AtomicUsize::new(0)),
            pts: Arc::new(AtomicF64::new(0.0)),
            last_read: Arc::new(AtomicInstant::now()),
            seek: Arc::new(AtomicF64::new(-1.0)),

            latency: None,
            skips: 0,
        }
    }

    pub fn available(&self) -> usize {
        self.view.size.load(Ordering::SeqCst)
    }

    pub fn remaining_space(&self) -> usize {
        self.view.remaining()
    }

    pub fn buffered(&self) -> f64 {
        (self.view.size() as f64) / (self.sample_rate as f64 * self.channels as f64)
    }

    pub fn read_to(&mut self, dest: &mut [i16]) -> usize {
        let to_copy = dest.len().min(self.view.size());
        self.samples_read.fetch_add(to_copy, Ordering::SeqCst);
        self.last_read.set_now(Ordering::SeqCst);
        for i in 0..to_copy {
            dest[i] = self.buffer[self.read_index];
            self.read_index = (self.read_index + 1) % self.buffer.len();
        }
        self.view.size.fetch_sub(to_copy, Ordering::SeqCst);
        to_copy
    }

    pub fn write_from(&mut self, src: &[i16], pts: f64, serial: u32) -> usize {
        if self.view.serial() != serial {
            self.view.serial.store(serial, Ordering::SeqCst);
            self.pts.store(pts, Ordering::SeqCst);
            self.last_read.set_now(Ordering::SeqCst);
            self.samples_read.store(0, Ordering::SeqCst);
            self.view.size.store(0, Ordering::SeqCst);
            self.read_index = 0;
            self.write_index = 0;
        }

        let to_copy = src.len().min(self.remaining_space());

        let time = to_copy as f64 / (self.sample_rate as f64 * self.channels as f64);
        let seek: f64 = self.seek.load(Ordering::SeqCst);
        if seek >= 0.0 {
            if pts < seek && pts + time < seek {
                self.pts.store(pts + time, Ordering::SeqCst);
                self.last_read.set_now(Ordering::SeqCst);
                self.skips += 1;
                return to_copy;
            } else {
                self.seek.store(-1.0f64, Ordering::SeqCst);
                self.skips = 0;
            }
        }

        for i in 0..to_copy {
            self.buffer[self.write_index] = src[i];
            self.write_index = (self.write_index + 1) % self.buffer.len();
        }
        self.view.size.fetch_add(to_copy, Ordering::SeqCst);
        to_copy
    }

    pub fn close(&mut self) {
        self.view.closed();
    }

    pub fn view(&self) -> AudioRingView {
        self.view.clone()
    }

    pub fn clock(&self) -> AudioRingClock {
        AudioRingClock {
            serial: self.view.serial.clone(),
            pts: self.pts.clone(),
            samples_read: self.samples_read.clone(),
            last_read: self.last_read.clone(),
            closed: self.view.closed.clone(),
            seek: self.seek.clone(),
            sample_rate: self.sample_rate,
            channels: self.channels,
        }
    }
}

pub struct AudioRingClock {
    serial: Arc<AtomicU32>,
    pts: Arc<AtomicF64>,
    samples_read: Arc<AtomicUsize>,
    last_read: Arc<AtomicInstant>,
    closed: Arc<AtomicBool>,
    seek: Arc<AtomicF64>,
    sample_rate: u32,
    channels: u16,
}

impl Clock for AudioRingClock {
    fn serial(&self) -> u32 {
        self.serial.load(Ordering::SeqCst)
    }

    fn pts(&self) -> f64 {
        let pts: f64 = self.pts.load(Ordering::SeqCst);
        let read_secs = self.samples_read.load(Ordering::SeqCst) as f64 / (self.sample_rate as f64 * self.channels as f64);
        pts + read_secs
    }

    fn pts_interpolated(&self) -> f64 {
        let pts: f64 = self.pts.load(Ordering::SeqCst);
        let read_secs = self.samples_read.load(Ordering::SeqCst) as f64 / (self.sample_rate as f64 * self.channels as f64);
        let inter = self.last_read.elapsed(Ordering::SeqCst);
        pts + read_secs + inter.as_secs_f64().max(0.0)
    }

    fn set_seek_flag(&self, seek: f64) {
        self.seek.store(seek, Ordering::SeqCst);
    }

    fn is_ext(&self) -> bool {
        false
    }

    fn sync_ext(&self, pts: f64) {}
}

#[derive(Clone)]
pub enum FrameConsumer {
    AudioConsumer(Arc<RwLock<AudioRingBuffer>>, AudioRingView),
    VideoConsumer(Arc<RwLock<FrameQueue>>, FrameQueueView),
}

impl FrameConsumer {
    pub fn unwrap_video(self) -> (Arc<RwLock<FrameQueue>>, FrameQueueView) {
        match self {
            FrameConsumer::VideoConsumer(queue, view) => (queue, view),
            _ => panic!("Frame consumer is not video!"),
        }
    }

    pub fn unwrap_audio(self) -> (Arc<RwLock<AudioRingBuffer>>, AudioRingView) {
        match self {
            FrameConsumer::AudioConsumer(ring, view) => (ring, view),
            _ => panic!("Frame consumer is not audio!"),
        }
    }

    pub fn is_closed(&self) -> bool {
        match self {
            FrameConsumer::VideoConsumer(frame_queue, view) => {
                view.closed()
            },
            FrameConsumer::AudioConsumer(ring, view) => {
                view.closed()
            }
        }
    }
}

pub enum DecodeWorkerMessage {
    End,
    Wakeup,
    Job(DecodeJob)
}

pub struct DecodeJob {
    decoder: Decoder,
    packet_queue: Arc<RwLock<PacketQueue>>,
    packet_queue_view: PacketQueueView,
    input_handle: InputJobHandle,
    frame_consumer: FrameConsumer,
    frame: Frame,
    audio_frame: Option<AudioFrame>,
    audio_frame_remaining: usize,
    audio_converter: Option<AudioConverter>,
    needs_input: bool
}

impl DecodeJob {
    pub fn consumer_has_space_and_serial(&self, serial: Option<u32>) -> bool {
        match &self.frame_consumer {
            FrameConsumer::AudioConsumer(ring, view) => {
                let remaining_space = view.remaining();
                if let Some(frame) = self.audio_frame.as_ref() {
                    if remaining_space >= self.audio_frame_remaining || frame.serial != serial {
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            },
            FrameConsumer::VideoConsumer(queue, view) => {
                if Some(view.serial()) != serial {
                    return true
                }
                view.remaining_space() > 0
            },
        }
    }

    pub fn write_received(&mut self) {
        match &mut self.frame_consumer {
            FrameConsumer::AudioConsumer(ring, view) => {
                let audio_frame = self.audio_frame.get_or_insert_with(|| AudioFrame::new());
                let context = self.audio_converter.as_mut().unwrap();
                context.convert_frame(&self.frame, audio_frame).unwrap();
                self.audio_frame_remaining = audio_frame.num_samples;

                let mut ring = ring.write().unwrap();
                let plane = audio_frame.plane(0);
                let begin_index = plane.len() - self.audio_frame_remaining;
                let end_index = begin_index + self.audio_frame_remaining;
                self.audio_frame_remaining -=
                    ring.write_from(&plane[begin_index..end_index], self.frame.pts.unwrap(), self.frame.serial.unwrap())
            }
            FrameConsumer::VideoConsumer(queue, view) => {
                let mut queue = queue.write().unwrap();
                if let Some(dest) = queue.peek_write(self.frame.serial.unwrap()) {
                    if self.decoder.is_hardware {
                        self.frame.transfer_hw_data_to(dest).unwrap();
                    } else {
                        self.frame.move_to(dest)
                    }
                    queue.push();
                } else {
                }
            }
        }
    }

    /// Will return whether the decoder should skip decoding this time
    pub fn write_remaining(&mut self) -> bool {
        match &mut self.frame_consumer {
            FrameConsumer::AudioConsumer(ring, view) => {
                if self.audio_frame_remaining == 0 {
                    return false;
                }
                let audio_frame = self.audio_frame.get_or_insert_with(|| AudioFrame::new());
                let mut ring = ring.write().unwrap();
                let plane = audio_frame.plane(0);
                let begin_index = plane.len() - self.audio_frame_remaining;
                let end_index = begin_index + self.audio_frame_remaining;
                self.audio_frame_remaining -= ring.write_from(&plane[begin_index..end_index], audio_frame.pts.unwrap(), audio_frame.serial.unwrap());
                self.audio_frame_remaining > 0
            }
            FrameConsumer::VideoConsumer(queue, view) => {
                false
            }
        }
    }
}

struct DecodeWorkerContext {
    jobs: Vec<DecodeJob>,
    receiver: Receiver<DecodeWorkerMessage>,
    close: bool,
    passes: Arc<AtomicUsize>,
}

impl DecodeWorkerContext {

    fn run(&mut self) {
        loop {
            loop {
                self.clear_jobs();
                if self.do_pass() && !self.handle_queued_messages() {
                    break
                }
            }
            self.await_wakeup();
        }
    }

    fn do_pass(&mut self) -> bool {
        let mut available = self.jobs.iter_mut()
            .filter(|job| {
                let view = &job.packet_queue_view;
                let frame_queue_space = job.consumer_has_space_and_serial(view.serial());
                let packet_queue_space = view.queued().unwrap_or(0.0);
                if job.needs_input {
                    frame_queue_space && packet_queue_space > 0.0
                } else {
                    frame_queue_space
                }
            })
            .peekable();
        self.passes.fetch_add(1, Ordering::Relaxed);
        let empty = available.peek().is_none();
        for job in available {
            if job.write_remaining() {
                continue;
            }
            match job.decoder.receive_frame(&mut job.frame) {
                DecoderResult::Error(error) => {
                    println!("Error decoding frame: {:?}", error);
                },
                DecoderResult::FrameReceived => {
                    job.write_received();
                }
                DecoderResult::NeedsInput => {
                    if let Some(packet) = job.packet_queue.write().unwrap().pop() {
                        job.decoder.send_packet(&packet).unwrap();
                        job.needs_input = false;
                    } else {
                        job.needs_input = true;
                        job.input_handle.notify_worker();
                    }
                }
            }
        }
        empty
    }

    fn await_wakeup(&mut self) -> bool {
        if let Some(msg) = self.receiver.recv().ok() {
            return self.handle_message(msg)
        }
        false
    }

    fn handle_queued_messages(&mut self) -> bool {
        let mut current = false;
        while let Some(msg) = self.receiver.try_recv().ok() {
            if self.handle_message(msg) {
                current = true;
            }
        }
        current
    }

    /// handles message and returns whether a pass should be done
    fn handle_message(&mut self, message: DecodeWorkerMessage) -> bool {
        match message {
            DecodeWorkerMessage::Wakeup => {
                true
            }
            DecodeWorkerMessage::Job(job) => {
                self.jobs.push(job);
                true
            }
            DecodeWorkerMessage::End => {
                self.close = true;
                false
            }
        }
    }

    fn clear_jobs(&mut self) {
        self.jobs.retain(|job| {
            let closed = job.frame_consumer.is_closed();
            if closed {
                job.packet_queue.write().unwrap().close();
                job.input_handle.notify_worker();
                println!("Decoder closed!")
            }
            !closed
        });
    }

}

pub struct DecodeWorker {
    sender: Sender<DecodeWorkerMessage>,
    pub passes: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl DecodeWorker {
    pub fn new() -> DecodeWorker {
        let (sender, receiver) = mpsc::channel();
        let passes = Arc::new(AtomicUsize::new(0));

        let mut context = DecodeWorkerContext {
            jobs: vec![],
            receiver,
            close: false,
            passes: passes.clone()
        };
        let thread = Some(std::thread::spawn(move || {
            context.run();
        }));

        DecodeWorker {
            sender,
            passes,
            thread,
        }
    }

    pub fn get_sender(&self) -> Sender<DecodeWorkerMessage> {
        self.sender.clone()
    }

    pub fn add_decode_job(&mut self, stream: &Stream, audio_config: Option<(u32, u16)>, input_job_handle: &InputJobHandle) -> (FrameConsumer, Sender<DecodeWorkerMessage>) {
        let queue = Arc::new(RwLock::new(PacketQueue::new(stream)));
        let sender = self.sender.clone();
        let job = match stream.stream_type {
            StreamType::Audio => {
                let (sample_rate, channels) = audio_config.expect("Missing audio config");
                let ring = AudioRingBuffer::new(0.5, sample_rate, channels);
                let view = ring.view();
                let frame_queue = AudioConsumer(Arc::new(RwLock::new(ring)), view);
                let converter = AudioConverter::new(channels as u32, sample_rate, AV_SAMPLE_FMT_S16);
                let packet_queue_view = queue.read().unwrap().view();
                DecodeJob {
                    decoder: Decoder::new(stream, &[]).unwrap(),
                    packet_queue: queue,
                    input_handle: input_job_handle.clone(),
                    packet_queue_view,
                    frame: Frame::new(),
                    frame_consumer: frame_queue,
                    audio_converter: Some(converter),
                    audio_frame: Some(AudioFrame::new()),
                    audio_frame_remaining: 0,
                    needs_input: true
                }
            },
            StreamType::Video => {
                let frame_queue = FrameQueue::new(7);
                let view = frame_queue.view();
                let frame_queue = VideoConsumer(Arc::new(RwLock::new(frame_queue)), view);
                let packet_queue_view = queue.read().unwrap().view();
                DecodeJob {
                    decoder: Decoder::new(stream, &[]).unwrap(),
                    packet_queue: queue,
                    input_handle: input_job_handle.clone(),
                    packet_queue_view,
                    frame: Frame::new(),
                    frame_consumer: frame_queue,
                    audio_converter: None,
                    audio_frame: None,
                    audio_frame_remaining: 0,
                    needs_input: true
                }
            },
            _ => unimplemented!(),
        };
        let queue = job.packet_queue.clone();
        input_job_handle.attach_queue(stream, queue, sender.clone());
        let consumer = job.frame_consumer.clone();
        sender.send(DecodeWorkerMessage::Job(job)).unwrap();
        (consumer, sender)
    }
}