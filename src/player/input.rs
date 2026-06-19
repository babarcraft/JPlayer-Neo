use crate::ffmpeg::error::Error;
use crate::ffmpeg::input::{Input, Stream};
use crate::ffmpeg::packet::{Packet, PACKET_COUNTER};
use crate::player::cache::{ByteBuffer, Cache, CacheReader, CacheWorker, CacheWorkerNotifier, SegmentView};
use crate::player::decoder::DecodeWorkerMessage;
use ffmpeg_sys_next::AV_NOPTS_VALUE;
use std::collections::VecDeque;
use std::mem::transmute;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, RwLock, RwLockReadGuard};
use std::thread::JoinHandle;

/// Serves as a normal queue, with the only difference being that it counts the duration of packets from the lowest to the highest instead of just the packet count.
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
        self.initialized.load(Ordering::SeqCst)
    }

    fn update(&self, begin: Option<f64>, end: Option<f64>, serial: Option<u32>) {
        if !self.is_initialized() {
            self.initialized.store(true, Ordering::SeqCst);
        }
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
            let begin = Some(pts).take_if(|_| !self.view.is_initialized() || set_serial.is_some());
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

    pub fn capacity(&self) -> usize {
        self.queue.capacity()
    }

    pub fn close(&mut self) {
        self.view.closed.store(true, Ordering::SeqCst);
    }
}

pub enum InputWorkerMessage {
    End,
    Update,
    Job(InputReadJob),
}

pub enum InputCommand {
    Begin,
    Seek(f64, f64, Option<i32>),
    PutQueue(usize, Arc<RwLock<PacketQueue>>, Sender<DecodeWorkerMessage>),
}

struct InputStreamEntry {
    queue: Arc<RwLock<PacketQueue>>,
    queue_view: PacketQueueView,
    consumer_notifier: Sender<DecodeWorkerMessage>,
}

enum JobInput {
    Input(Input),
    Cache(CacheReader, Option<CacheWorkerNotifier>)
}

impl JobInput {

    fn seek(&mut self, min: f64, time: f64, stream: Option<i32>) -> Result<(), Error> {
        match self {
            JobInput::Input(input) => input.seek(min, time, stream),
            JobInput::Cache(cache, notifier) => {
                cache.seek(time);
                if let Some(notifier) = notifier {
                    notifier.notify();
                }
                Ok(())
            }
        }
    }

}

struct InputReadJob {
    input: JobInput,
    entries: Vec<Option<InputStreamEntry>>,
    receiver: Receiver<InputCommand>,
    begin: bool,
}

impl InputReadJob {
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
    cache_worker: CacheWorker,
    pub passes: Arc<AtomicUsize>,
}

struct InputWorkerContext {
    receiver: Receiver<InputWorkerMessage>,
    inputs: Vec<InputReadJob>,
    passes: Arc<AtomicUsize>,
    buffer: ByteBuffer,
    http_client: Option<reqwest::blocking::Client>,
    close: bool,
}

pub struct TaskHandle<T: Send, E: Send> {
    value: Option<Result<T, E>>,
    receiver: Receiver<Result<T, E>>,
    cancel: Arc<AtomicBool>,
}

impl<T: Send, E: Send + std::fmt::Debug> TaskHandle<T, E> {

    pub fn new() -> (TaskHandle<T, E>, Arc<AtomicBool>, Sender<Result<T, E>>) {
        let (sender, receiver) = mpsc::channel::<Result<T, E>>();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        (TaskHandle {
            value: None,
            receiver,
            cancel,
        }, cancel_clone, sender)
    }

    pub fn poll(&mut self) -> bool {
        if self.value.is_some() {
            return true
        }
        if let Some(value) = self.receiver.try_recv().ok() {
            self.value = Some(value);
            return true
        }
        false
    }

    pub fn take(mut self) -> Result<T, E> {
        self.value.take().unwrap()
    }
}

impl<T: Send, E: Send> Drop for TaskHandle<T, E> {
    fn drop(&mut self) {
        self.cancel.store(false, Ordering::Relaxed);
    }
}

impl InputWorkerContext {

    fn run(&mut self) {
        loop {
            loop {
                self.clear_inputs();
                self.handle_input_commands();
                let queued = self.handle_queued_messages();
                let pass = self.do_pass();
                if pass && !queued {
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

    fn get_http_client(&mut self) -> &mut reqwest::blocking::Client {
        self.http_client.get_or_insert_with(|| reqwest::blocking::Client::new())
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
            .filter(|pair| pair.begin)
            .filter_map(|pair| {
                let queue_has_space = pair.min_queued()
                    .and_then(|q| Some(q < 5.0))
                    .unwrap_or(true);
                let should = match &mut pair.input {
                    JobInput::Input(input) => {
                        let serial = input.serial;
                        let error = input.read_error;
                        (queue_has_space || !pair.serial_matches(serial)) && !error
                    }
                    JobInput::Cache(cache, _) => {
                        let sealed = cache.is_sealed();
                        let duration = cache.cached_duration();
                        let serial = cache.serial;
                        let serial_matches = pair.serial_matches(serial);
                        (queue_has_space || !serial_matches) && (duration >= 5.0 || (sealed && duration > 0.0))
                    }
                };
                if should {
                    Some(pair)
                } else {
                    None
                }
            }).peekable();
        let empty = available.peek().is_none();
        for pair in available {
            let needs_input = pair.min_queued()
                .and_then(|q| Some(q < 5.0))
                .unwrap_or(true);
            match &mut pair.input {
                JobInput::Input(input) => {
                    match input.read_packet() {
                        Ok(packet) => {
                            if let Some(entry) = &pair.entries[packet.stream_index() as usize] {
                                let mut queue = entry.queue.write().unwrap();
                                queue.push(packet);
                                entry.consumer_notifier.send(DecodeWorkerMessage::Wakeup).unwrap();
                            }
                        },
                        Err(error) => {
                            eprintln!("Read error: {:?}", error);
                        }
                    }
                }
                JobInput::Cache(cache, _) => {
                    let packet = cache.read_packet();
                    match packet {
                        Ok(packet) => {
                            if let Some(entry) = &pair.entries[packet.stream_index() as usize] {
                                let mut queue = entry.queue.write().unwrap();
                                queue.push(packet);
                                println!("Queued: {:02.2} {:02} Total packets {:04}", queue.queued().unwrap_or(0.0), queue.capacity(), PACKET_COUNTER.load(Ordering::Relaxed));
                                entry.consumer_notifier.send(DecodeWorkerMessage::Wakeup).unwrap();
                            }
                        }
                        Err(err) => {
                            eprintln!("Cache Read error: {:?}", err);
                        }
                    }
                }
            }
        }
        empty
    }

    fn clear_inputs(&mut self) {
        self.inputs.retain(|pair| pair.entries.iter().all(|queue| {
            if let Some(entry) = queue {
                let closed = entry.queue_view.closed();
                !closed
            } else {
                true
            }
        }));
        self.inputs.sort_by(|pair_a, pair_b| {
            let a_min = pair_a.min_queued().unwrap_or(0.0);
            let b_min = pair_b.min_queued().unwrap_or(0.0);
            b_min.total_cmp(&a_min)
        });
    }

    fn handle_input_commands(&mut self) {
        for job in self.inputs.iter_mut() {
            let input = &mut job.input;
            let mut seek = None;
            while let Some(command) = job.receiver.try_recv().ok() {
                match command {
                    InputCommand::Seek(min, max, stream) => {
                        seek = Some((min, max, stream));
                    },
                    InputCommand::PutQueue(index, queue, sender) => {
                        let queue_view = queue.read().unwrap().view();
                        job.entries[index] = Some(InputStreamEntry {
                            queue,
                            queue_view,
                            consumer_notifier: sender,
                        });
                    }
                    InputCommand::Begin => {
                        job.begin = true;
                    }
                }
            }
            if let Some((min, max, stream)) = seek {
                match input.seek(min, max, stream) {
                    Err(error) => {
                        eprintln!("Error seeking: {:?}", error);
                    }
                    _ => {},
                }
            }
        }
    }
}

pub struct InputWorkerNotifier {
    sender: Sender<InputWorkerMessage>,
}

impl InputWorkerNotifier {

    pub fn notify(&self) {
        self.sender.send(InputWorkerMessage::Update).unwrap();
    }

}

#[derive(Clone)]
pub struct InputJobHandle {
    job_sender: Sender<InputCommand>,
    worker_sender: Sender<InputWorkerMessage>,
    cache_views: Option<Arc<RwLock<Vec<SegmentView>>>>
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

    pub fn notify_begin(&self) {
        self.job_sender.send(InputCommand::Begin).unwrap();
        self.notify_worker();
    }

    pub fn cache_segments(&'_ self) -> Option<RwLockReadGuard<'_, Vec<SegmentView>>> {
        self.cache_views.as_ref().and_then(|cache| cache.read().ok())
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
            http_client: None,
            buffer: ByteBuffer::new(1024),
            passes: passes.clone(),
        };
        let thread = Some(std::thread::spawn(move || {
            context.run();
        }));

        InputWorker {
            thread,
            sender,
            passes,
            cache_worker: CacheWorker::new()
        }
    }

    pub fn notifier(&self) -> InputWorkerNotifier {
        InputWorkerNotifier {
            sender: self.sender.clone(),
        }
    }
    
    pub fn add_pre_cached(&mut self, cache: CacheReader) -> InputJobHandle {
        let (sender, receiver) = mpsc::channel::<InputCommand>();

        let queue = (0..cache.streams.len()).map(|_| None).collect();
        let views = cache.segment_views.clone();
        let job = InputReadJob {
            input: JobInput::Cache(cache, None),
            entries: queue,
            receiver,
            begin: false
        };
        let worker_sender = self.sender.clone();
        worker_sender.send(InputWorkerMessage::Job(job)).unwrap();
        InputJobHandle {
            job_sender: sender,
            worker_sender,
            cache_views: Some(views)
        }
    }

    pub fn add_input<P: AsRef<Path>>(&mut self, input: Input, cache_path: Option<P>) -> InputJobHandle {
        let (sender, receiver) = mpsc::channel::<InputCommand>();

        let queue = (0..input.streams.len()).map(|_| None).collect();
        let (input, views) = if let Some(path) = cache_path {
            let cache = Cache::new(path, input);
            let views = cache.views();
            let (cache, notifier) = self.cache_worker.push(cache, self.notifier());
            (JobInput::Cache(cache, Some(notifier)), Some(views))
        } else {
            (JobInput::Input(input), None)
        };
        let job = InputReadJob {
            input,
            entries: queue,
            receiver,
            begin: false
        };
        let worker_sender = self.sender.clone();
        worker_sender.send(InputWorkerMessage::Job(job)).unwrap();
        InputJobHandle {
            job_sender: sender,
            worker_sender,
            cache_views: views
        }
    }
}