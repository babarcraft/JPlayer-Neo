use std::cell::Cell;
use std::collections::VecDeque;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex, RwLock};
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
    sender: Sender<DecodeWorkerMessage>,
    input: Arc<Mutex<Input>>,
    queue: Arc<RwLock<PacketQueue>>,
}

pub struct InputWorker {
    sender: Sender<InputWorkerMessage>,
    inputs: Arc<Mutex<Vec<InputPair>>>,
    thread: Option<JoinHandle<()>>,
}

impl InputWorker {
    pub fn new() -> InputWorker {
        let (sender, receiver) = mpsc::channel();
        let inputs: Arc<Mutex<Vec<InputPair>>> = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let inputs = inputs.clone();
            Some(std::thread::spawn(move || {
                loop {
                    let mut inputs = inputs.lock().unwrap();
                    todo!()
                }
            }))
        };

        InputWorker {
            thread,
            inputs,
            sender,
        }
    }
}