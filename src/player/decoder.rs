use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use crate::ffmpeg::decode::{Decoder, DecoderResult};
use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::Stream;
use crate::player::input::PacketQueue;
use crate::player::surface::FrameQueue;

pub enum DecodeWorkerMessage {
    End,
    Wakeup
}

pub struct DecodePair {
    decoder: Decoder,
    packet_queue: Arc<RwLock<PacketQueue>>,
    frame_queue: Arc<RwLock<FrameQueue>>,
}

pub struct DecodeWorker {
    decoders: Arc<Mutex<Vec<DecodePair>>>,
    sender: Sender<DecodeWorkerMessage>,
    thread: Option<JoinHandle<()>>,
}

impl DecodeWorker {
    pub fn new() -> DecodeWorker {
        let (sender, receiver) = mpsc::channel();
        let decoders: Arc<Mutex<Vec<DecodePair>>> = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let decoders = decoders.clone();
            Some(std::thread::spawn(move || {
                let mut frame = Frame::new();
                loop {
                    let mut decoders = decoders.lock().unwrap();
                    let mut available = decoders.iter_mut()
                        .filter(|pair| pair.frame_queue.read().unwrap().has_space())
                        .peekable();
                    let frame_queue_available = !available.peek().is_some();
                    for pair in available {
                        match pair.decoder.receive_frame(&mut frame) {
                            DecoderResult::Error(error) => {
                            },
                            DecoderResult::FrameReceived => {
                                let mut queue = pair.frame_queue.write().unwrap();
                                if let Some(dest) = queue.peek_write() {
                                    frame.transfer_hw_data_to(dest).unwrap();
                                    queue.push();
                                }
                            }
                            DecoderResult::NeedsInput => {
                                let mut queue = pair.packet_queue.write().unwrap();
                                if let Some(packet) = queue.pop() {
                                    pair.decoder.send_packet(&packet).unwrap();
                                }
                            }
                        }
                    }
                    drop(decoders);
                    if !frame_queue_available {
                        let message = receiver.recv().unwrap();
                        if let DecodeWorkerMessage::End = message {
                            return
                        }
                    }
                }
            }))
        };
        
        DecodeWorker {
            decoders,
            sender,
            thread,
        }
    }
    
    pub fn begin_stream_decode(&mut self, stream: &Stream) -> (Sender<DecodeWorkerMessage>, Arc<RwLock<PacketQueue>>, Arc<RwLock<FrameQueue>>) {
        let decoder = Decoder::new(&stream, vec![]).unwrap();
        let packet_queue = Arc::new(RwLock::new(PacketQueue::new(stream)));
        let frame_queue = Arc::new(RwLock::new(FrameQueue::new(15)));
        self.decoders.lock().unwrap().push(DecodePair {
            decoder,
            packet_queue: packet_queue.clone(),
            frame_queue: frame_queue.clone()
        });
        (self.sender.clone(), packet_queue, frame_queue)
    }
}