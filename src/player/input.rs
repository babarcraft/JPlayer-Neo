use std::cell::Cell;
use std::collections::VecDeque;
use std::fs::File;
use std::{fs, io};
use std::io::BufWriter;
use std::mem::transmute;
use std::ops::Deref;
use std::path::Path;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use ffmpeg_sys_next::{register_t, AVERROR_EOF, AV_NOPTS_VALUE};
use glfw::GamepadButton::ButtonB;
use reqwest::Response;
use crate::ffmpeg;
use crate::ffmpeg::error::Error;
use crate::ffmpeg::input::{Input, Stream};
use crate::ffmpeg::packet::{ByteBuffer, Packet};
use crate::gs::texture::Texture;
use crate::player::cache::{Cache, CacheError};
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
            let begin = if self.view.is_initialized() && set_serial.is_none() {
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

pub type IoOrHttpError = (Option<reqwest::Error>, Option<io::Error>);

pub enum InputWorkerMessage {
    End,
    Update,
    Job(InputReadJob),

    HttpGetText(String, Sender<Result<String, reqwest::Error>>),
    HttpGetDownload(String, String, Sender<Result<(), IoOrHttpError>>),
    FileRead(String, Sender<Result<Vec<u8>, io::Error>>),
    OpenInput(String, Vec<(String, String)>, Sender<Result<Input, Error>>)
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
    Cache(Cache)
}

impl JobInput {

    fn serial(&self) -> u32 {
        match self {
            JobInput::Input(input) => input.serial,
            JobInput::Cache(cache) => cache.serial
        }
    }

    fn input(&self) -> &Input {
        match self {
            JobInput::Input(input) => input,
            JobInput::Cache(cache) => &cache.input
        }
    }

    fn has_error(&self) -> bool {
        match self {
            JobInput::Input(input) => input.read_error,
            JobInput::Cache(cache) => cache.has_error()
        }
    }

    fn seek(&mut self, min: f64, time: f64, stream: Option<i32>) -> Result<(), Error> {
        match self {
            JobInput::Input(input) => input.seek(min, time, stream),
            JobInput::Cache(cache) => match cache.seek(time) {
                Ok(()) => Ok(()),
                Err(error) => match error {
                    CacheError::SourceReadError(error) => Err(error),
                    _ => panic!("Cache seek error {:?}", error)
                }
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

    pub fn can_cache(&self) -> bool {
        if let JobInput::Cache(cache) = &self.input {
            !cache.input.read_error
        } else {
            false
        }
    }

    fn run_cache(&mut self) -> Result<(), CacheError> {
        match &mut self.input {
            JobInput::Input(_) => Ok(()),
            JobInput::Cache(cache) => {
                cache.write_next()
            }
        }
    }

    fn read_packet(&mut self) -> Result<Packet, Error> {
        match &mut self.input {
            JobInput::Input(input) => {
                input.read_packet()
            }
            JobInput::Cache(cache) => {
                let packet = cache.read_packet();
                match packet {
                    Ok(packet) => Ok(packet),
                    Err(error) => {
                        match error {
                            CacheError::SourceReadError(error) => Err(error),
                            CacheError::Eof => Err(Error::from_code(AVERROR_EOF)),
                            _ => panic!("Cache error: {:?}", error)
                        }
                    }
                }
            }
        }
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
            InputWorkerMessage::HttpGetText(path, sender) => {
                let client = self.get_http_client();
                let result = client.get(&path).send()
                    .map(|result| result.text().unwrap());
                sender.send(result).unwrap();
                true
            }
            InputWorkerMessage::HttpGetDownload(path, filepath, sender) => {
                let client = self.get_http_client();
                let result = client.get(&path).send();
                if result.is_err() {
                    sender.send(Err((result.err(), None))).unwrap();
                    return true
                }
                let mut result = result.unwrap();
                let path = Path::new(&path);

                if let Some(parent) = path.parent() {
                    let result = fs::create_dir_all(parent);
                    if result.is_err() {
                        sender.send(Err((None, result.err()))).unwrap();
                    }
                }

                let file = File::create(filepath);
                if file.is_err() {
                    sender.send(Err((None, file.err()))).unwrap();
                    return true
                }
                let mut file = BufWriter::new(file.unwrap());
                result.copy_to(&mut file).unwrap();
                sender.send(Ok(())).unwrap();
                true
            }
            InputWorkerMessage::FileRead(path, sender) => {
                let result = std::fs::read(path);
                sender.send(result).unwrap();
                true
            }
            InputWorkerMessage::OpenInput(path, options, sender) => {
                let options = options.iter()
                    .map(|(key, val)| (key.as_str(), val.as_str()))
                    .collect::<Vec<_>>();
                let input = Input::open(path.as_str(), &options);
                sender.send(input).unwrap();
                true
            }
        }
    }

    fn do_pass(&mut self) -> bool {
        let mut available = self.inputs.iter_mut()
            .filter(|pair| pair.begin)
            .filter(|pair| {
                let has_error = pair.input.has_error();
                if has_error {
                    println!("Has Error")
                }
                !has_error
            })
            .filter(|pair| {
                    let queue_has_space = pair.min_queued()
                        .and_then(|q| Some(q < 5.0))
                        .unwrap_or(true);
                    let serial_changed = !pair.serial_matches(pair.input.serial());
                    let can_cache = pair.can_cache();
                    queue_has_space || serial_changed || can_cache
                }).peekable();
        let empty = available.peek().is_none();
        for pair in available {
            if let Some(err) = pair.run_cache().err() {
                match err {
                    CacheError::SourceReadError(err) => println!("Source read error {:?}", err),
                    _ => println!("Error while reading {:?}", err)
                }
            }
            if !pair.min_queued()
                .and_then(|q| Some(q < 5.0))
                .unwrap_or(true) {
                continue;
            }
            match pair.read_packet() {
                Ok(packet) => {
                    if let Some(entry) = &pair.entries[packet.stream_index() as usize] {
                        let mut queue = entry.queue.write().unwrap();
                        self.passes.fetch_add(1, Ordering::Relaxed);
                        queue.push(packet);
                        entry.consumer_notifier.send(DecodeWorkerMessage::Wakeup).unwrap();
                    }
                },
                Err(error) => {
                    eprintln!("Read error: {:?}", error);
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

    pub fn notify_begin(&self) {
        self.job_sender.send(InputCommand::Begin).unwrap();
        self.notify_worker();
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
        }
    }

    pub fn get_sender(&self) -> Sender<InputWorkerMessage> {
        self.sender.clone()
    }

    pub fn add_input(&mut self, input: Input) -> InputJobHandle {
        let (sender, receiver) = mpsc::channel::<InputCommand>();

        let queue = (0..input.streams.len()).map(|_| None).collect();
        let job = InputReadJob {
            input: JobInput::Cache(Cache::new("test.bin", input)),
            entries: queue,
            receiver,
            begin: false
        };
        let worker_sender = self.sender.clone();
        worker_sender.send(InputWorkerMessage::Job(job)).unwrap();
        InputJobHandle {
            job_sender: sender,
            worker_sender
        }
    }

    pub fn add_http_get_text(&mut self, path: String) -> TaskHandle<String, reqwest::Error> {
        let (handler, cancel, sender) = TaskHandle::new();
        self.sender.send(InputWorkerMessage::HttpGetText(path, sender)).unwrap();
        handler
    }
    
    pub fn add_open_input(&mut self, path: String, options: Vec<(String, String)>) -> TaskHandle<Input, Error> {
        let (handler, cancel, sender) = TaskHandle::new();
        self.sender.send(InputWorkerMessage::OpenInput(path, options, sender)).unwrap();
        handler
    }

    pub fn add_http_get_download(&mut self, url: String, path: String) -> Receiver<Result<(), IoOrHttpError>> {
        let (sender, receiver) = mpsc::channel();
        self.sender.send(InputWorkerMessage::HttpGetDownload(url, path, sender)).unwrap();
        receiver
    }

    pub fn add_file_read(&mut self, path: String) -> Receiver<Result<Vec<u8>, io::Error>> {
        let (sender, receiver) = mpsc::channel();
        self.sender.send(InputWorkerMessage::FileRead(path, sender)).unwrap();
        receiver
    }
}