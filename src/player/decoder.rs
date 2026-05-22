use std::collections::HashMap;
use std::slice::Iter;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Instant;
use ffmpeg_sys_next::AVSampleFormat::AV_SAMPLE_FMT_S16;
use ffmpeg_sys_next::register_t;
use crate::ffmpeg::decode::{AudioConverter, Decoder, DecoderResult};
use crate::ffmpeg::frame::{AudioFrame, Frame};
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::player::clock::{Clock, GenClock};
use crate::player::decoder::FrameConsumer::{AudioConsumer, VideoConsumer};
use crate::player::input::{InputCommand, InputWorker, InputWorkerMessage, PacketQueue};
use crate::player::surface::FrameQueue;

pub struct AudioRingBuffer {
    buffer: Vec<i16>,
    write_index: usize,
    read_index: usize,
    size: usize,

    serial: Option<u32>,
    pts: Option<f64>,
    samples_read: usize,
    last_read: Option<Instant>,

    pub seek: Option<f64>,

    pub latency: Option<f64>,

    pub sample_rate: u32,
    pub channels: u16,

    pub skips: usize,

    pub closed: bool,
}

impl AudioRingBuffer {
    pub fn new(seconds: f32, sample_rate: u32, channels: u16) -> AudioRingBuffer {
        let size = (seconds * (sample_rate as f32) * (channels as f32)).round() as usize;
        AudioRingBuffer {
            sample_rate,
            channels,
            size,
            buffer: vec![0i16; size],
            write_index: 0,
            read_index: 0,
            samples_read: 0,
            pts: None,
            seek: None,
            serial: None,
            latency: None,
            last_read: None,
            skips: 0,
            closed: false,
        }
    }

    pub fn available(&self) -> usize {
        self.size
    }

    pub fn remaining_space(&self) -> usize {
        self.buffer.len() - self.size
    }

    pub fn buffered(&self) -> f64 {
        (self.size as f64) / (self.sample_rate as f64 * self.channels as f64)
    }

    pub fn read_to(&mut self, dest: &mut [i16]) -> usize {
        let to_copy = dest.len().min(self.size);
        self.samples_read += to_copy;
        self.last_read = Some(Instant::now());
        for i in 0..to_copy {
            dest[i] = self.buffer[self.read_index];
            self.read_index = (self.read_index + 1) % self.buffer.len();
            self.size -= 1
        }
        to_copy
    }

    pub fn write_from(&mut self, src: &[i16], pts: f64, serial: u32) -> usize {
        if let Some(current_serial) = self.serial {
            if current_serial != serial {
                self.serial = Some(serial);
                self.pts = Some(pts);

                self.last_read = None;
                self.samples_read = 0;
                self.size = 0;
                self.read_index = 0;
                self.write_index = 0;
            }
        } else {
            self.serial = Some(serial);
            self.pts = Some(pts);
        }

        let to_copy = src.len().min(self.remaining_space());

        let time = to_copy as f64 / (self.sample_rate as f64 * self.channels as f64);
        if let Some(seek) = self.seek {
            if pts < seek && pts + time < seek {
                self.pts = Some(pts);
                self.last_read = None;
                self.skips += 1;
                return to_copy;
            } else {
                self.seek = None;
                self.skips = 0;
            }
        }

        for i in 0..to_copy {
            self.buffer[self.write_index] = src[i];
            self.write_index = (self.write_index + 1) % self.buffer.len();
            self.size += 1
        }
        to_copy
    }

    pub fn close(&mut self) {
        self.closed = true;
    }
}

impl Clock for AudioRingBuffer {
    fn serial(&self) -> Option<u32> {
        self.serial
    }

    fn pts_interpolated(&self) -> Option<f64> {
        let pts = self.pts?;
        let read_secs = self.samples_read as f64 / (self.sample_rate as f64 * self.channels as f64);
        let inter = self.last_read.clone().unwrap_or(Instant::now()).elapsed().as_secs_f64();
        Some(pts + read_secs + inter + self.latency.unwrap_or(0.0))
    }
}

#[derive(Clone)]
pub enum FrameConsumer {
    AudioConsumer(Arc<RwLock<AudioRingBuffer>>),
    VideoConsumer(Arc<RwLock<FrameQueue>>),
}

impl FrameConsumer {
    pub fn unwrap_video(self) -> Arc<RwLock<FrameQueue>> {
        match self {
            FrameConsumer::VideoConsumer(consumer) => consumer,
            _ => panic!("Frame consumer is not video!"),
        }
    }

    pub fn unwrap_audio(self) -> Arc<RwLock<AudioRingBuffer>> {
        match self {
            FrameConsumer::AudioConsumer(consumer) => consumer,
            _ => panic!("Frame consumer is not audio!"),
        }
    }

    pub fn is_closed(&self) -> bool {
        match self {
            FrameConsumer::VideoConsumer(frame_queue) => {
                frame_queue.read().unwrap().closed
            },
            FrameConsumer::AudioConsumer(ring) => {
                ring.read().unwrap().closed
            }
        }
    }
}

pub enum DecodeWorkerMessage {
    End,
    Wakeup(&'static str)
}

pub struct DecodePair {
    decoder: Decoder,
    packet_queue: (Arc<RwLock<PacketQueue>>, Sender<InputWorkerMessage>),
    frame_consumer: FrameConsumer,
    audio_frame: Option<AudioFrame>,
    audio_frame_remaining: usize,
    audio_converter: Option<AudioConverter>,
    needs_input: bool
}

impl DecodePair {
    pub fn consumer_has_space_and_serial(&self, serial: Option<u32>) -> bool {
        match &self.frame_consumer {
            FrameConsumer::AudioConsumer(ring) => {
                let remaining_space = {
                    let ring = ring.read().unwrap();
                    ring.remaining_space()
                };
                if let Some(_) = self.audio_frame.as_ref() {
                    if remaining_space >= self.audio_frame_remaining {
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            },
            FrameConsumer::VideoConsumer(queue) => {
                let queue = queue.read().unwrap();
                if queue.serial() != serial {
                    println!("Damn: {} != {}", serial.unwrap_or(0), queue.serial().unwrap_or(0));
                    return true
                }
                queue.has_space()
            },
        }
    }

    pub fn write_received(&mut self, frame: &Frame) {
        match &mut self.frame_consumer {
            FrameConsumer::AudioConsumer(ring) => {
                let audio_frame = self.audio_frame.get_or_insert_with(|| AudioFrame::new());
                let context = self.audio_converter.as_mut().unwrap();
                context.convert_frame(frame, audio_frame).unwrap();
                self.audio_frame_remaining = audio_frame.num_samples;

                let mut ring = ring.write().unwrap();
                let plane = audio_frame.plane(0);
                let begin_index = plane.len() - self.audio_frame_remaining;
                let end_index = begin_index + self.audio_frame_remaining;
                self.audio_frame_remaining -=
                    ring.write_from(&plane[begin_index..end_index], frame.pts.unwrap(), frame.serial.unwrap())
            }
            FrameConsumer::VideoConsumer(queue) => {
                let mut queue = queue.write().unwrap();
                if let Some(dest) = queue.peek_write(frame.serial.unwrap()) {
                    if self.decoder.is_hardware {
                        frame.transfer_hw_data_to(dest).unwrap();
                    } else {
                        frame.move_to(dest)
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
            FrameConsumer::AudioConsumer(ring) => {
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
            FrameConsumer::VideoConsumer(queue) => {
                false
            }
        }
    }
}

pub struct DecodeWorker {
    decoders: Arc<Mutex<Vec<DecodePair>>>,
    sender: Sender<DecodeWorkerMessage>,
    pub passes: Arc<AtomicUsize>,
    pub wake_ups: Arc<RwLock<HashMap<&'static str, usize>>>,
    thread: Option<JoinHandle<()>>,
}

impl DecodeWorker {
    pub fn new() -> DecodeWorker {
        let (sender, receiver) = mpsc::channel();
        let passes = Arc::new(AtomicUsize::new(0));
        let decoders: Arc<Mutex<Vec<DecodePair>>> = Arc::new(Mutex::new(Vec::new()));
        let wake_ups = Arc::new(RwLock::new(HashMap::new()));
        let thread = {
            let decoders = decoders.clone();
            let passes = passes.clone();
            let wake_ups = wake_ups.clone();
            Some(std::thread::spawn(move || {
                let mut frame = Frame::new();
                loop {
                    loop {
                        let mut decoders = decoders.lock().unwrap();
                        decoders.retain(|pair| {
                            let closed = pair.frame_consumer.is_closed();
                            if closed {
                                pair.packet_queue.0.write().unwrap().close();
                                pair.packet_queue.1.send(InputWorkerMessage::Update).unwrap();
                                println!("Decoder closed!")
                            }
                            !closed
                        });
                        let mut available = decoders.iter_mut()
                            .filter(|pair| {
                                let packet_queue = pair.packet_queue.0.read().unwrap();
                                let frame_queue_space = pair.consumer_has_space_and_serial(packet_queue.serial());
                                let packet_queue_space = packet_queue.queued().unwrap_or(0.0);
                                if pair.needs_input {
                                    frame_queue_space && packet_queue_space > 0.0
                                } else {
                                    frame_queue_space
                                }
                            })
                            .peekable();
                        let empty = available.peek().is_none();
                        for pair in available {
                            if pair.write_remaining() {
                                continue;
                            }
                            match pair.decoder.receive_frame(&mut frame) {
                                DecoderResult::Error(error) => {
                                    println!("Error decoding frame: {:?}", error);
                                },
                                DecoderResult::FrameReceived => {
                                    pair.write_received(&frame);
                                }
                                DecoderResult::NeedsInput => {
                                    if let Some(packet) = pair.packet_queue.0.write().unwrap().pop() {
                                        pair.decoder.send_packet(&packet).unwrap();
                                        pair.packet_queue.1.send(InputWorkerMessage::Update).unwrap();
                                        pair.needs_input = false;
                                    } else {
                                        pair.needs_input = true;
                                        passes.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                        if empty {
                            break
                        }
                    }
                    let message = receiver.recv().unwrap();
                    match message {
                        DecodeWorkerMessage::End => {
                            return
                        }
                        DecodeWorkerMessage::Wakeup(name) => {
                            let mut map = wake_ups.write().unwrap();
                            if let Some(current) = map.get_mut(name) {
                                *current += 1;
                            } else {
                                map.insert(name, 1usize);
                            }
                        }
                    }
                }
            }))
        };

        DecodeWorker {
            decoders,
            sender,
            wake_ups,
            passes,
            thread,
        }
    }

    pub fn get_sender(&self) -> Sender<DecodeWorkerMessage> {
        self.sender.clone()
    }

    pub fn begin_decode(&mut self, streams: &Vec<Option<&Stream>>, audio_config: Option<(u32, u16)>, input: Input, input_worker: &mut InputWorker) -> (Vec<Option<(Sender<DecodeWorkerMessage>, FrameConsumer)>>, Sender<InputCommand>) {
        let queues = streams.iter().map(|stream| {
            if let Some(stream) = stream {
                Some((*stream, self.sender.clone()))
            } else {
                None
            }
        }).collect::<Vec<_>>();
        let (queues, worker_sender, input_sender) = input_worker.add_input(input, queues);
        let mut decoders = self.decoders.lock().unwrap();
        let mut out_queues = (0..streams.len()).map(|_| None).collect::<Vec<_>>();
        for (queue, stream) in queues.iter().zip(streams) {
            let stream = match stream {
                Some(stream) => *stream,
                None => continue,
            };
            let queue = match queue {
                Some(queue) => queue,
                None => continue,
            };

            let pair = match stream.stream_type {
                StreamType::Audio => {
                    let (sample_rate, channels) = audio_config.expect("Missing audio config");
                    let frame_queue = AudioConsumer(Arc::new(RwLock::new(AudioRingBuffer::new(0.5, sample_rate, channels))));
                    let converter = AudioConverter::new(channels as u32, sample_rate, AV_SAMPLE_FMT_S16);
                    DecodePair {
                        decoder: Decoder::new(stream, vec![]).unwrap(),
                        packet_queue: (queue.clone(), worker_sender.clone()),
                        frame_consumer: frame_queue,
                        audio_converter: Some(converter),
                        audio_frame: Some(AudioFrame::new()),
                        audio_frame_remaining: 0,
                        needs_input: true
                    }
                },
                StreamType::Video => {
                    let frame_queue = VideoConsumer(Arc::new(RwLock::new(FrameQueue::new(15))));
                    DecodePair {
                        decoder: Decoder::new(stream, vec![]).unwrap(),
                        packet_queue: (queue.clone(), worker_sender.clone()),
                        frame_consumer: frame_queue,
                        audio_converter: None,
                        audio_frame: None,
                        audio_frame_remaining: 0,
                        needs_input: true
                    }
                },
                _ => continue,
            };
            out_queues[stream.index as usize] = Some((self.sender.clone(), pair.frame_consumer.clone()));
            decoders.push(pair);

        }
        (out_queues, input_sender)
    }
}