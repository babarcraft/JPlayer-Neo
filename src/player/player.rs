use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::sync::mpsc::Sender;
use std::time::Instant;
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::gs::texture::InternalFormat;
use crate::player::audio::AudioDevice;
use crate::player::clock::Clock;
use crate::player::decoder::{DecodeWorker, DecodeWorkerMessage};
use crate::player::input::{InputCommand, InputWorker};
use crate::player::surface::{FrameQueue, VideoSurface};

pub struct VideoPlayback {
    frame_queue: Arc<RwLock<FrameQueue>>,
    master_clock: Box<dyn Clock>,
    sender: Sender<DecodeWorkerMessage>,
    surface: Rc<RefCell<VideoSurface>>,
    pub last_pts: Option<f64>,
    last_duration: f64,
    frame_timer: Option<f64>,
    serial: Option<u32>,
    pub seek: Option<f64>,
    pub playing: bool,
    begin: Instant,
    pub(crate) skips: usize,
}

impl VideoPlayback {
    pub fn new(
        frame_queue: Arc<RwLock<FrameQueue>>,
        sender: Sender<DecodeWorkerMessage>,
        surface: Rc<RefCell<VideoSurface>>,
        master_clock: Box<dyn Clock>
    ) -> Self {
        Self {
            frame_queue,
            sender,
            master_clock,
            last_pts: None,
            playing: false,
            last_duration: 0.0,
            serial: None,
            frame_timer: None,
            seek: None,
            begin: Instant::now(),
            surface,
            skips: 0
        }
    }

    pub fn update(&mut self) {
        if !self.playing {
            return;
        }

        let current_time = self.begin.elapsed().as_secs_f64();

        let audio_clock = self.master_clock.pts_interpolated();

        let mut video_surface = self.surface.borrow_mut();

        let mut should_pop = false;
        let mut queue_was_full = false;

        if let Some(queue) = self.frame_queue.try_read().ok() {
            if let Some(frame_serial) = queue.serial() {
                if let Some(serial) = self.serial {
                    if serial != frame_serial {
                        self.serial = Some(frame_serial);
                        self.frame_timer.take();
                        self.last_pts.take();
                    }
                } else {
                    self.serial = Some(frame_serial);
                }
            }
            queue_was_full = !queue.has_space();

            if let Some(frame) = queue.peek_read() {
                let current_pts = frame.pts.unwrap_or(0.0) as f64;
                let mut skip = false;

                if let Some(seek) = self.seek {
                    if current_pts < seek.min(audio_clock) {
                        self.skips += 1;
                        self.last_pts = Some(current_pts);
                        skip = true;
                        should_pop = true;
                    } else {
                        self.seek = None;
                        self.skips = 0;
                    }
                }

                if !skip {
                    if self.frame_timer.is_none() || self.last_pts.is_none() {
                        self.frame_timer = Some(current_time);
                        self.last_pts = Some(current_pts);
                        self.last_duration = frame.duration.unwrap_or(0.04) as f64;

                        video_surface.upload(frame, &[
                            InternalFormat::R(8),
                            InternalFormat::Rg(8),
                        ], Some(2));
                        video_surface.convert_output();
                        should_pop = true;
                    } else {
                        let last_pts = self.last_pts.unwrap();

                        let (duration, diff, delay) = Self::calculate_delay(self.last_duration, audio_clock, current_pts, last_pts);

                        let target_time = self.frame_timer.unwrap() + delay;

                        if current_time < target_time {
                        } else {
                            self.frame_timer = Some(target_time);

                            if current_time - target_time > 0.1 {
                                self.frame_timer = Some(current_time);
                            }

                            if diff < -0.1 {
                                should_pop = true;
                            } else if diff < 0.4 {
                                video_surface.upload(frame, &[
                                    InternalFormat::R(8),
                                    InternalFormat::Rg(8),
                                ], Some(2));
                                video_surface.convert_output();
                                self.last_pts = Some(current_pts);
                                self.last_duration = duration;
                                should_pop = true;
                            }
                        }
                    }
                }
            }
        }

        drop(video_surface);

        if should_pop {
            self.pop_and_notify(queue_was_full);
        }
    }

    fn calculate_delay(last_duration: f64, audio_clock: f64, current_pts: f64, last_pts: f64) -> (f64, f64, f64) {
        let mut duration = current_pts - last_pts;
        if duration <= 0.0 || duration > 1.0 {
            duration = last_duration;
        }

        let diff = current_pts - audio_clock;
        let sync_threshold = 0.04_f64.max(0.1_f64.min(duration));

        let mut delay = duration;
        if diff.abs() < 3600.0 {
            if diff <= -sync_threshold {
                delay = 0.0_f64.max(duration + diff);
            } else if diff >= sync_threshold {
                delay = duration + diff;
            }
        }
        (duration, diff, delay)
    }

    fn pop_and_notify(&mut self, queue_was_full: bool) {
        if let Some(mut write_queue) = self.frame_queue.try_write().ok() {
            write_queue.pop();
            if queue_was_full {
                let _ = self.sender.send(DecodeWorkerMessage::Wakeup("Notify frame queue pop"));
            }
        }
    }
}

impl Drop for VideoPlayback {
    fn drop(&mut self) {
        self.frame_queue.write().unwrap().close();
        self.sender.send(DecodeWorkerMessage::Wakeup("Notify queue drop")).unwrap();
    }
}

pub struct VideoPlayer {
    pub video_playback: Option<VideoPlayback>,
    pub audio_device: Option<AudioDevice>,
    pub video_stream: Option<Stream>,
    pub audio_stream: Option<Stream>,
    pub master_clock: Box<dyn Clock>,
    pub estimated_duration: f64,

    input_command_sender: Sender<InputCommand>
}

impl VideoPlayer {
    pub fn new(input: Input, video_surface: Option<Rc<RefCell<VideoSurface>>>, decode_worker: &mut DecodeWorker, input_worker: &mut InputWorker) -> VideoPlayer {
        let audio_stream = input
            .streams
            .iter()
            .find(|stream| stream.stream_type == StreamType::Audio)
            .cloned();
        let video_stream = input
            .streams
            .iter()
            .find(|stream| stream.stream_type == StreamType::Video)
            .cloned();
        let mut playback_clock: Option<Box<dyn Clock>> = None;
        let mut master_clock: Option<Box<dyn Clock>> = None;
        let mut queues: Vec<Option<&Stream>> = (0..input.streams.len()).map(|_| None).collect();
        if let Some(video_stream) = &video_stream {
            queues[video_stream.index as usize] = Some(video_stream);
        }
        if let Some(audio_stream) = &audio_stream {
            queues[audio_stream.index as usize] = Some(audio_stream);
        }

        let estimated_duration = input.duration();

        let (mut decode_streams, command_sender) = decode_worker.begin_decode(&queues, Some((44100, 1)), input, input_worker);

        let audio_device = if let Some(audio_stream) = &audio_stream {
            Some({
                let (sender, queue) = decode_streams[audio_stream.index as usize].take().unwrap();
                let ring_buffer = queue.unwrap_audio();
                {
                    let ring = ring_buffer.read().unwrap();
                    playback_clock = Some(Box::new(ring.clock()));
                    master_clock = Some(Box::new(ring.clock()));
                }
                AudioDevice::default_device(ring_buffer, sender).unwrap()
            })
        } else {
            todo!()
        };
        let video_playback = if let Some(video_stream) = &video_stream && let Some(video_surface) = video_surface {
            let (sender, queue) = decode_streams[video_stream.index as usize].take().unwrap();
            let playback = VideoPlayback::new(queue.unwrap_video(), sender, video_surface.clone(), playback_clock.unwrap());
            Some(playback)
        } else { None };

        VideoPlayer {
            video_playback,
            audio_device,
            video_stream,
            audio_stream,
            master_clock: master_clock.unwrap(),
            input_command_sender: command_sender,
            estimated_duration,
        }
    }

    pub fn render_update(&mut self) {
        if let Some(playback) = self.video_playback.as_mut() {
            playback.update();
        }
    }

    pub fn play(&mut self) {
        if let Some(device) = &mut self.audio_device {
            device.play();
        }
        if let Some(playback) = self.video_playback.as_mut() {
            playback.playing = true;
        }
    }

    pub fn pause(&mut self) {
        if let Some(device) = &mut self.audio_device {
            device.pause();
        }
        if let Some(playback) = self.video_playback.as_mut() {
            playback.playing = false;
        }
    }

    pub fn seek(&mut self, target: f64) {
        self.input_command_sender.send(InputCommand::Seek(0.0, target, None)).unwrap();
        self.master_clock.set_seek_flag(target);
        if let Some(playback) = &mut self.video_playback {
            playback.seek = Some(target);
        }
    }

    pub fn current_pts(&self) -> f64 {
        if self.is_playing() {
            self.master_clock.pts_interpolated()
        } else {
            self.master_clock.pts()
        }
    }

    pub fn is_playing(&self) -> bool {
        if let Some(device) = &self.audio_device {
            return device.is_playing()
        }
        if let Some(playback) = self.video_playback.as_ref() {
            return playback.playing
        }
        false
    }
}