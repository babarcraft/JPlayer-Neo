use std::slice::Iter;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::AtomicUsize;
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
use crate::player::input::{InputWorker, InputWorkerMessage, PacketQueue};
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
    
    pub latency: Option<f64>,

    pub sample_rate: u32,
    pub channels: u16,
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
            serial: None,
            latency: None,
            last_read: None,
        }
    }

    pub fn available(&self) -> usize {
        self.size
    }

    pub fn remaining_space(&self) -> usize {
        self.buffer.len() - self.size
    }

    pub fn read(&mut self, dest: &mut [i16]) -> usize {
        let to_copy = dest.len().min(self.size);
        for i in 0..to_copy {
            dest[i] = self.buffer[self.read_index];
            self.read_index = (self.read_index + 1) % self.buffer.len();
            self.size -= 1
        }
        self.samples_read += to_copy;
        to_copy
    }

    pub fn write(&mut self, src: &[i16], pts: f64, serial: u32) -> usize {
        if let Some(current_serial) = self.serial {
            if current_serial != serial {
                self.serial = Some(current_serial);
                self.pts = Some(pts);

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
        for i in 0..to_copy {
            self.buffer[self.write_index] = src[i];
            self.write_index = (self.write_index + 1) % self.buffer.len();
            self.size += 1
        }
        to_copy
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
}

pub enum DecodeWorkerMessage {
    End,
    Wakeup
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
    pub fn consumer_has_space(&self) -> bool {
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
            FrameConsumer::VideoConsumer(queue) => queue.read().unwrap().has_space(),
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
                self.audio_frame_remaining -= ring.write(&plane[begin_index..end_index], frame.pts.unwrap(), frame.serial.unwrap())
            }
            FrameConsumer::VideoConsumer(queue) => {
                let mut queue = queue.write().unwrap();
                if let Some(dest) = queue.peek_write() {
                    if self.decoder.is_hardware {
                        frame.transfer_hw_data_to(dest).unwrap();
                    } else {
                        frame.move_to(dest)
                    }
                    queue.push();
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
                self.audio_frame_remaining -= ring.write(&plane[begin_index..end_index], audio_frame.pts.unwrap(), audio_frame.serial.unwrap());
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
    thread: Option<JoinHandle<()>>,
}

impl DecodeWorker {
    pub fn new() -> DecodeWorker {
        let (sender, receiver) = mpsc::channel();
        let passes = Arc::new(AtomicUsize::new(0));
        let decoders: Arc<Mutex<Vec<DecodePair>>> = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let decoders = decoders.clone();
            let passes = passes.clone();
            Some(std::thread::spawn(move || {
                let mut frame = Frame::new();
                loop {
                    loop {
                        let mut decoders = decoders.lock().unwrap();
                        let mut available = decoders.iter_mut()
                            .filter(|pair| {
                                let frame_queue_space = pair.consumer_has_space();
                                let packet_queue_space = pair.packet_queue.0.read().unwrap().queued().unwrap_or(0.0);
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
                                    passes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    if let Some(packet) = pair.packet_queue.0.write().unwrap().pop() {
                                        pair.decoder.send_packet(&packet).unwrap();
                                        pair.packet_queue.1.send(InputWorkerMessage::Update).unwrap();
                                        pair.needs_input = false;
                                    } else {
                                        pair.needs_input = true;
                                    }
                                }
                            }
                        }
                        if empty {
                            break
                        }
                    }
                    let message = receiver.recv().unwrap();
                    if let DecodeWorkerMessage::End = message {
                        return
                    }
                }
            }))
        };

        DecodeWorker {
            decoders,
            sender,
            passes,
            thread,
        }
    }

    pub fn begin_decode(&mut self, streams: &Vec<Option<&Stream>>, audio_config: Option<(u32, u16)>, input: Input, input_worker: &mut InputWorker) -> Vec<Option<(Sender<DecodeWorkerMessage>, FrameConsumer)>> {
        let queues = streams.iter().map(|stream| {
            if let Some(stream) = stream {
                Some((*stream, self.sender.clone()))
            } else {
                None
            }
        }).collect::<Vec<_>>();
        let (queues, sender) = input_worker.add_input(input, queues);
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
                        packet_queue: (queue.clone(), sender.clone()),
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
                        packet_queue: (queue.clone(), sender.clone()),
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
        out_queues
    }
}