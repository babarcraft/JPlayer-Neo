use std::cell::Cell;
use std::collections::VecDeque;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use ffmpeg_sys_next::{AVERROR_EOF, AV_NOPTS_VALUE};
use crate::ffmpeg::error::Error;
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
    serial: Option<u32>,
    closed: bool,
}

impl PacketQueue {
    pub fn new(stream: &Stream) -> Self {
        Self {
            queue: VecDeque::new(),
            begin_pts: None,
            end_pts: None,
            serial: None,
            timebase: stream.timebase,
            closed: false,
        }
    }

    pub fn serial(&self) -> Option<u32> {
        self.serial
    }
    
    pub fn push(&mut self, packet: Packet) {
        if let Some(serial) = self.serial {
            if serial != packet.serial {
                self.queue.clear();
                self.serial = Some(packet.serial);
            }
        } else {
            self.serial = Some(packet.serial);
        }
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

    pub fn close(&mut self) {
        self.closed = true;
    }
}

pub enum InputWorkerMessage {
    End,
    Update
}

pub enum InputCommand {
    Seek(f64, f64, Option<i32>)
}

pub struct InputPair {
    input: Input,
    queue: Vec<Option<(Arc<RwLock<PacketQueue>>, Sender<DecodeWorkerMessage>)>>,
    receiver: Receiver<InputCommand>,
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

    pub fn serial_matches(&self, serial: u32) -> bool {
        for (queue, _) in self.queue.iter()
            .filter(|item| item.is_some())
            .map(|item| item.as_ref().unwrap()) {

            let queue = queue.read().unwrap();
            match queue.serial {
                Some(current) => if current == serial {
                    continue;
                }
                None => {
                    return true;
                }
            }

            return false
        }
        true
    }

    pub fn min_queued(&self) -> Option<f64> {
        let mut current = None;
        for (queue, _) in self.queue.iter()
            .filter(|item| item.is_some())
            .map(|item| item.as_ref().unwrap()) {

            let queue = queue.read().unwrap();
            let queued = queue.queued();
            if queued.is_none() {
                continue;
            }
            let queued = queued.unwrap();

            if let Some(current_queued) = current {
                if queued > current_queued {
                    current = Some(queued)
                }
            } else {
                current = Some(queued);
            }
        }
        current
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
                        inputs.retain(|pair| pair.queue.iter().all(|queue| {
                            if let Some((queue, sender)) = queue {
                                let closed = queue.read().unwrap().closed;
                                if closed {
                                    println!("Packet queue closed!")
                                }
                                !closed
                            } else {
                                true
                            }
                        }));
                        inputs.sort_by(|pair_a, pair_b| {
                            let a_min = pair_a.min_queued().unwrap_or(0.0);
                            let b_min = pair_b.min_queued().unwrap_or(0.0);
                            a_min.total_cmp(&b_min)
                        });
                        for pair in inputs.iter_mut() {
                            let input = &mut pair.input;
                            while let Some(command) = pair.receiver.try_recv().ok() {
                                match command {
                                    InputCommand::Seek(min, max, stream) => {
                                        match input.seek(min, max, stream) {
                                            Err(error) => {
                                                println!("Error seeking: {:?}", error);
                                            }
                                            _ => eprintln!("Currently not implemented!"),
                                        }
                                    }
                                }
                            }
                        }

                        let mut available = inputs.iter_mut()
                            .filter(|pair| !pair.input.eof)
                            .filter(|pair|
                                pair.min_queued()
                                    .and_then(|q| Some(q < 5.0))
                                    .unwrap_or(true) ||
                                !pair.serial_matches(pair.input.serial)
                            ).peekable();
                        passes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let empty = available.peek().is_none();
                        for pair in available {
                            match pair.input.read_packet() {
                                Ok(packet) => {
                                    if let Some((queue, sender)) = &pair.queue[packet.stream_index() as usize] {
                                        let mut queue = queue.write().unwrap();
                                        let last = queue.queued().unwrap_or(0.0);
                                        queue.push(packet);
                                        sender.send(DecodeWorkerMessage::Wakeup("New input")).unwrap();
                                    }
                                },
                                Err(error) => {
                                    eprintln!("Read error: {:?}", error);
                                }
                            }
                            if pair.input.eof {
                                println!("Eof but what the hell?")
                            }
                        }
                        if empty {
                            break
                        } else {
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

    pub fn get_sender(&self) -> Sender<InputWorkerMessage> {
        self.sender.clone()
    }

    pub fn add_input(&mut self,
                     input: Input,
                     mut queues: Vec<Option<(&Stream, Sender<DecodeWorkerMessage>)>>
    ) -> (Vec<Option<Arc<RwLock<PacketQueue>>>>, Sender<InputWorkerMessage>, Sender<InputCommand>) {
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
        let (sender, receiver) = mpsc::channel::<InputCommand>();
        inputs.push(InputPair {
            input,
            queue: pair_queues,
            receiver
        });
        self.sender.send(InputWorkerMessage::Update).unwrap();

        (queues, self.sender.clone(), sender)
    }
}