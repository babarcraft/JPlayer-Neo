use std::cell::Cell;
use std::collections::VecDeque;
use std::mem::transmute;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use ffmpeg_sys_next::{register_t, AVERROR_EOF, AV_NOPTS_VALUE};
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
    view: PacketQueueView,
}

#[derive(Clone)]
pub struct PacketQueueView {
    initialized: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    begin_pts: Arc<AtomicU64>,
    end_pts: Arc<AtomicU64>,
    serial: Arc<AtomicU32>,
}

impl PacketQueueView {

    fn new() -> PacketQueueView {
        PacketQueueView {
            initialized: Arc::new(AtomicBool::new(false)),
            closed: Arc::new(AtomicBool::new(false)),
            begin_pts: Arc::new(AtomicU64::new(0)),
            end_pts: Arc::new(AtomicU64::new(0)),
            serial: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn queued(&self) -> Option<f64> {
        if self.is_initialized() {
            let begin: f64 = unsafe { transmute(self.begin_pts.load(Ordering::SeqCst)) };
            let end: f64 = unsafe { transmute(self.end_pts.load(Ordering::SeqCst)) };
            Some(end - begin)
        } else {
            None
        }
    }

    pub fn serial(&self) -> Option<u32> {
        if self.is_initialized() {
            Some(self.serial.load(Ordering::SeqCst))
        } else {
            None
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Relaxed)
    }

    fn update(&self, begin: Option<f64>, end: Option<f64>, serial: Option<u32>) {
        if let Some(begin) = begin {
            self.begin_pts.store(unsafe { transmute(begin) }, Ordering::SeqCst);
        }
        if let Some(end) = end {
            self.end_pts.store(unsafe { transmute(end) }, Ordering::SeqCst);
        }
        if let Some(serial) = serial {
            self.serial.store(serial, Ordering::SeqCst);
        }
    }

}

impl PacketQueue {
    pub fn new(stream: &Stream) -> Self {
        Self {
            queue: VecDeque::new(),
            timebase: stream.timebase,
            view: PacketQueueView::new(),
        }
    }

    pub fn serial(&self) -> Option<u32> {
        self.view.serial()
    }

    pub fn view(&self) -> PacketQueueView {
        self.view.clone()
    }
    
    pub fn push(&mut self, packet: Packet) {
        let serial = self.view.serial();
        let set_serial = if let Some(serial) = serial {
            if serial != packet.serial {
                self.queue.clear();
                Some(packet.serial)
            } else {
                None
            }
        } else {
            Some(packet.serial)
        };
        let pts = packet.pts();
        if pts != AV_NOPTS_VALUE {
            let pts = self.timebase * (pts as f64);
            let begin = if self.view.is_initialized() {
                None
            } else { Some(pts) };
            self.view.update(begin, Some(pts), set_serial);
        }
        self.queue.push_back(packet);
    }
    
    pub fn pop(&mut self) -> Option<Packet> {
        if let Some(packet) = self.queue.pop_front() {
            let pts = packet.pts();
            if pts != AV_NOPTS_VALUE {
                let pts = self.timebase * (pts as f64);
                self.view.update(Some(pts), None, None);
            }
            Some(packet)
        } else {
            None
        }
    }

    pub fn queued(&self) -> Option<f64> {
        self.view.queued()
    }

    pub fn close(&mut self) {
        self.view.closed.store(true, Ordering::SeqCst);
    }
}

pub enum InputWorkerMessage {
    End,
    Update,
    Job(InputJob)
}

pub enum InputCommand {
    Seek(f64, f64, Option<i32>),
    PutQueue(usize, Arc<RwLock<PacketQueue>>, Sender<DecodeWorkerMessage>),
}

struct InputJobEntry {
    queue: Arc<RwLock<PacketQueue>>,
    queue_view: PacketQueueView,
    consumer_notifier: Sender<DecodeWorkerMessage>,
}

struct InputJob {
    input: Input,
    entries: Vec<Option<InputJobEntry>>,
    receiver: Receiver<InputCommand>,
}

impl InputJob {
    pub fn serial_matches(&self, serial: u32) -> bool {
        for entry in self.entries.iter()
            .filter(|item| item.is_some())
            .map(|item| item.as_ref().unwrap()) {

            if let Some(current) = entry.queue_view.serial() {
                if current == serial {
                    continue
                }
            } else {
                return true
            }

            return false
        }
        true
    }

    pub fn min_queued(&self) -> Option<f64> {
        let mut current = None;
        for entry in self.entries.iter()
            .filter(|item| item.is_some())
            .map(|item| item.as_ref().unwrap()) {

            let queued = entry.queue_view.queued();
            if queued.is_none() { continue; }
            let queued = queued.unwrap();

            if let Some(current_queued) = current {
                if queued < current_queued {
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
    thread: Option<JoinHandle<()>>,
    pub passes: Arc<AtomicUsize>,
}

struct InputWorkerContext {
    receiver: Receiver<InputWorkerMessage>,
    inputs: Vec<InputJob>,
    passes: Arc<AtomicUsize>,
    close: bool,
}

impl InputWorkerContext {
    fn run(&mut self) {
        loop {
            loop {
                self.clear_inputs();
                self.handle_input_commands();
                if self.do_pass() && !self.handle_queued_messages() {
                    break;
                }
            }
            self.await_wakeup();
        }
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

    fn handle_message(&mut self, message: InputWorkerMessage) -> bool {
        match message {
            InputWorkerMessage::End => {
                self.close = true;
                false
            }
            InputWorkerMessage::Update => {
                true
            }
            InputWorkerMessage::Job(job) => {
                self.inputs.push(job);
                true
            }
        }
    }

    fn do_pass(&mut self) -> bool {
        let mut available = self.inputs.iter_mut()
            .filter(|pair| !pair.input.eof)
            .filter(|pair|
                pair.min_queued()
                    .and_then(|q| Some(q < 5.0))
                    .unwrap_or(true) ||
                    !pair.serial_matches(pair.input.serial)
            ).peekable();
        let empty = available.peek().is_none();
        self.passes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        for pair in available {
            match pair.input.read_packet() {
                Ok(packet) => {
                    if let Some(entry) = &pair.entries[packet.stream_index() as usize] {
                        let mut queue = entry.queue.write().unwrap();
                        // let last = queue.queued().unwrap_or(0.0);
                        queue.push(packet);
                        entry.consumer_notifier.send(DecodeWorkerMessage::Wakeup).unwrap();
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
        empty
    }

    fn clear_inputs(&mut self) {
        self.inputs.retain(|pair| pair.entries.iter().all(|queue| {
            if let Some(entry) = queue {
                !entry.queue_view.closed()
            } else {
                true
            }
        }));
        self.inputs.sort_by(|pair_a, pair_b| {
            let a_min = pair_a.min_queued().unwrap_or(0.0);
            let b_min = pair_b.min_queued().unwrap_or(0.0);
            a_min.total_cmp(&b_min)
        });
    }

    fn handle_input_commands(&mut self) {
        for job in self.inputs.iter_mut() {
            let input = &mut job.input;
            while let Some(command) = job.receiver.try_recv().ok() {
                match command {
                    InputCommand::Seek(min, max, stream) => {
                        match input.seek(min, max, stream) {
                            Err(error) => {
                                println!("Error seeking: {:?}", error);
                            }
                            _ => eprintln!("Currently not implemented!"),
                        }
                    },
                    InputCommand::PutQueue(index, queue, sender) => {
                        let queue_view = queue.read().unwrap().view();
                        job.entries[index] = Some(InputJobEntry {
                            queue,
                            queue_view,
                            consumer_notifier: sender,
                        });
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct InputJobHandle {
    job_sender: Sender<InputCommand>,
    worker_sender: Sender<InputWorkerMessage>,
}

impl InputJobHandle {
    
    pub fn seek(&self, min: f64, max: f64, stream: Option<&Stream>) {
        self.job_sender.send(InputCommand::Seek(min, max, stream.map(|st| st.index))).unwrap();
    }
    
    pub fn attach_queue(&self, stream: &Stream, queue: Arc<RwLock<PacketQueue>>, decode_sender: Sender<DecodeWorkerMessage>) {
        self.job_sender.send(InputCommand::PutQueue(stream.index as usize, queue, decode_sender)).unwrap();
    }
    
    pub fn notify_worker(&self) {
        self.worker_sender.send(InputWorkerMessage::Update).unwrap();
    }
    
}

impl InputWorker {

    pub fn new() -> InputWorker {
        let (sender, receiver) = mpsc::channel();
        let passes = Arc::new(AtomicUsize::new(0));

        let mut context = InputWorkerContext {
            receiver,
            inputs: vec![],
            close: false,
            passes: passes.clone(),
        };
        let thread = Some(std::thread::spawn(move || {
            context.run();
        }));

        InputWorker {
            thread,
            sender,
            passes,
        }
    }

    pub fn get_sender(&self) -> Sender<InputWorkerMessage> {
        self.sender.clone()
    }

    pub fn add_input(&mut self, input: Input) -> InputJobHandle {
        let (sender, receiver) = mpsc::channel::<InputCommand>();

        let queue = (0..input.streams.len()).map(|_| None).collect();
        let job = InputJob {
            input,
            entries: queue,
            receiver
        };
        let worker_sender = self.sender.clone();
        worker_sender.send(InputWorkerMessage::Job(job)).unwrap();
        InputJobHandle {
            job_sender: sender,
            worker_sender
        }
    }
}