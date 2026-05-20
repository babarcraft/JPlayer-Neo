use std::cell::Cell;
use std::collections::VecDeque;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use ffmpeg_sys_next::AV_NOPTS_VALUE;
use crate::ffmpeg::input::{Input, Stream};
use crate::ffmpeg::packet::Packet;
use crate::gs::texture::Texture;
use crate::player::decoder::DecodeWorkerMessage;

/// Serves as a normal queue, with the only difference being that it counts the duration of packets from the lowest to the highest instead of just a number.
#[derive(Clone)]
pub struct PacketQueue {
    queue: VecDeque<Packet>,
    timebase: f64,
    begin_pts: Option<f64>,
    end_pts: Option<f64>,
}

impl PacketQueue {
    pub fn new(stream: &Stream) -> Self {
        Self {
            queue: VecDeque::new(),
            begin_pts: None,
            end_pts: None,
            timebase: stream.timebase
        }
    }
    
    pub fn push(&mut self, packet: Packet) {
        let pts = packet.pts();
        if pts != AV_NOPTS_VALUE {
            let pts = self.timebase * (pts as f64);
            self.begin_pts.get_or_insert(pts);
            self.end_pts = Some(self.end_pts.get_or_insert(pts).max(pts));
        }
        self.queue.push_back(packet);
    }
    
    pub fn pop(&mut self) -> Option<Packet> {
        if let Some(packet) = self.queue.pop_front() {
            let pts = packet.pts();
            if pts != AV_NOPTS_VALUE {
                let pts = self.timebase * (pts as f64);
                self.begin_pts = Some(self.begin_pts?.max(pts));
            }
            Some(packet)
        } else {
            None
        }
    }
    
    pub fn queued(&self) -> Option<f64> {
        Some(self.end_pts? - self.begin_pts?)
    }
}

pub enum InputWorkerMessage {
    End,
    Update
}

pub struct InputPair {
    input: Arc<Mutex<Input>>,
    queue: Vec<Option<(Arc<RwLock<PacketQueue>>, Sender<DecodeWorkerMessage>)>>,
}

impl InputPair {
    pub fn average_queued(&self) -> f64 {
        let mut count = 0;
        for _ in self.queue.iter().filter(|this| this.is_some()) {
            count += 1;
        }
        let queued = self.queue.iter()
            .filter(|item| item.is_some())
            .map(|queue| queue.as_ref().unwrap().0.read().unwrap().queued())
            .filter(|item| item.is_some())
            .map(|item| item.unwrap());
        queued.sum::<f64>() / count as f64
    }
}

pub struct InputWorker {
    sender: Sender<InputWorkerMessage>,
    inputs: Arc<Mutex<Vec<InputPair>>>,
    thread: Option<JoinHandle<()>>,
    pub passes: Arc<AtomicUsize>,
}

impl InputWorker {
    pub fn new() -> InputWorker {
        let (sender, receiver) = mpsc::channel();
        let inputs: Arc<Mutex<Vec<InputPair>>> = Arc::new(Mutex::new(Vec::new()));
        let passes = Arc::new(AtomicUsize::new(0));
        let thread = {
            let inputs = inputs.clone();
            let passes = passes.clone();
            Some(std::thread::spawn(move || {
                loop {
                    loop {
                        let mut inputs = inputs.lock().unwrap();
                        inputs.sort_by(|pair_a, pair_b| {
                            pair_a.average_queued().total_cmp(&pair_b.average_queued())
                        });
                        let mut available = inputs.iter_mut()
                            .filter(|pair| pair.average_queued() < 5.0)
                            .peekable();
                        passes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let empty = available.peek().is_none();
                        for pair in inputs.iter_mut() {
                            let mut input = pair.input.lock().unwrap();
                            if let Some(packet) = input.read_packet().ok() {
                                if let Some((queue, sender)) = &pair.queue[packet.stream_index() as usize] {
                                    let mut queue = queue.write().unwrap();
                                    let last = queue.queued().unwrap_or(0.0);
                                    queue.push(packet);
                                    sender.send(DecodeWorkerMessage::Wakeup).unwrap();
                                }
                            }
                        }
                        if empty {
                            break
                        }
                    }
                    receiver.recv().unwrap();
                }
            }))
        };

        InputWorker {
            thread,
            inputs,
            passes,
            sender,
        }
    }

    pub fn add_input(&mut self, input: Input, mut queues: Vec<Option<(&Stream, Sender<DecodeWorkerMessage>)>>) -> (Vec<Option<Arc<RwLock<PacketQueue>>>>, Sender<InputWorkerMessage>) {
        let mut inputs = self.inputs.lock().unwrap();
        for _ in 0..(queues.len() - input.streams.len()) {
            queues.push(None);
        }
        let pair_queues = queues.iter_mut().map(|sender| {
            if let Some((stream, sender)) = sender.take() {
                Some((Arc::new(RwLock::new(PacketQueue::new(stream))), sender))
            } else {
                None
            }
        }).collect::<Vec<_>>();
        let queues = pair_queues.iter().map(|queue| {
            if let Some((queue, sender)) = queue {
                Some(queue.clone())
            } else {
                None
            }
        }).collect::<Vec<_>>();
        inputs.push(InputPair {
            input: Arc::new(Mutex::new(input)),
            queue: pair_queues,
        });
        self.sender.send(InputWorkerMessage::Update).unwrap();

        (queues, self.sender.clone())
    }
}