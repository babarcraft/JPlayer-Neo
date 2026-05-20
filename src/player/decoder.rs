use std::slice::Iter;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use crate::ffmpeg::decode::{AudioConverter, Decoder, DecoderResult};
use crate::ffmpeg::frame::{AudioFrame, Frame};
use crate::ffmpeg::input::{Input, Stream};
use crate::player::input::{InputWorker, InputWorkerMessage, PacketQueue};
use crate::player::surface::FrameQueue;

pub struct AudioRingBuffer {
    buffer: Vec<i16>,
    write_index: usize,
    read_index: usize,
    size: usize,
    serial: Option<u32>
}

impl AudioRingBuffer {
    pub fn new(size: usize) -> AudioRingBuffer {
        AudioRingBuffer {
            buffer: vec![0i16; size],
            write_index: 0,
            read_index: 0,
            size,
            serial: None
        }
    }

    pub fn available(&self) -> usize {
        self.size
    }

    pub fn read(&mut self, dest: &mut [i16]) -> usize {
        let to_copy = dest.len().min(self.size);
        for i in 0..to_copy {
            dest[i] = self.buffer[self.read_index];
            self.read_index = (self.read_index + 1) % self.buffer.len();
            self.size -= 1
        }
        to_copy
    }

    pub fn write(&mut self, src: &[i16], serial: u32) -> usize {
        if let Some(current_serial) = self.serial {
            if current_serial != serial {
                self.serial = Some(current_serial);
                self.size = 0;
                self.read_index = 0;
                self.write_index = 0;
            }
        } else {
            self.serial = Some(serial);
        }
        
        let to_copy = src.len().min(self.buffer.len() - self.size);
        for i in 0..to_copy {
            self.buffer[self.write_index] = src[i];
            self.write_index = (self.write_index + 1) % self.buffer.len();
            self.size += 1
        }
        to_copy
    }
    
    pub fn serial(&self) -> Option<u32> {
        self.serial
    }
}

#[derive(Clone)]
pub enum FrameConsumer {
    AudioConsumer(Arc<RwLock<AudioRingBuffer>>),
    VideoConsumer(Arc<RwLock<FrameQueue>>),
}

pub enum DecodeWorkerMessage {
    End,
    Wakeup
}

pub struct DecodePair {
    decoder: Decoder,
    packet_queue: (Arc<RwLock<PacketQueue>>, Sender<InputWorkerMessage>),
    frame_queue: Arc<RwLock<FrameQueue>>,
    audio_frame: Option<AudioFrame>,
    audio_converter: Option<AudioConverter>,
    needs_input: bool
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
                                let frame_queue_space = pair.frame_queue.read().unwrap().has_space();
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
                            match pair.decoder.receive_frame(&mut frame) {
                                DecoderResult::Error(error) => {
                                    println!("Error decoding frame: {:?}", error);
                                },
                                DecoderResult::FrameReceived => {
                                    let mut queue = pair.frame_queue.write().unwrap();
                                    if let Some(dest) = queue.peek_write() {
                                        if pair.decoder.is_hardware {
                                            frame.transfer_hw_data_to(dest).unwrap();
                                        } else {
                                            frame.move_to(dest)
                                        }
                                        queue.push();
                                    }
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

    pub fn begin_decode(&mut self, streams: &Vec<Option<&Stream>>, input: Input, input_worker: &mut InputWorker) -> Vec<Option<(Sender<DecodeWorkerMessage>, Arc<RwLock<FrameQueue>>)>> {
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
            let frame_queue = Arc::new(RwLock::new(FrameQueue::new(15)));
            decoders.push(DecodePair {
                decoder: Decoder::new(stream, vec![]).unwrap(),
                packet_queue: (queue.clone(), sender.clone()),
                frame_queue: frame_queue.clone(),
                audio_converter: None,
                audio_frame: None,
                needs_input: true
            });
            out_queues[stream.index as usize] = Some((self.sender.clone(), frame_queue));
        }
        out_queues
    }
}