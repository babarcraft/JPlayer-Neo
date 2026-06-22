use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use bytemuck::Contiguous;
use cpal::{BufferSize, ChannelCount, Device, Host, SampleFormat, Stream, StreamConfig};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ffmpeg_sys_next::{av_ripemd_init, AVSampleFormat};
use crate::ffmpeg::frame::SampleType;
use crate::player::decoder::{AudioRingBuffer, DecodeJobHandle, DecodeWorkerMessage};

pub struct AudioDevice {
    host: Host,
    device: Device,
    stream: Stream,
    handle: DecodeJobHandle,
    playing: Arc<AtomicBool>,
    pub volume: Arc<AtomicI16>,
    pub ring_buffer: Arc<RwLock<AudioRingBuffer>>,
}

impl AudioDevice {

    pub fn default_device(ring_buffer: Arc<RwLock<AudioRingBuffer>>, handle: DecodeJobHandle) -> Result<AudioDevice, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().unwrap();

        let config = {
            let buffer = ring_buffer.write().unwrap();
            let channels = buffer.channels;
            let sample_rate = buffer.sample_rate;
            let buffer_size = 128 * sample_rate * channels as u32;
            // buffer.latency = Some((buffer_size as f64 / sample_rate as f64));
            StreamConfig {
                channels,
                sample_rate,
                buffer_size: BufferSize::Default,
            }
        };

        let playing = Arc::new(AtomicBool::new(false));
        let volume = Arc::new(AtomicI16::new(100));

        let stream = {
            let ring_buffer = ring_buffer.clone();
            let view = ring_buffer.read().unwrap().view();
            let handle = handle.clone();
            let playing = playing.clone();
            let volume = volume.clone();
            device.build_output_stream(&config, move |output: &mut [i16], info: &cpal::OutputCallbackInfo| {
                let playing = playing.load(Ordering::Relaxed);
                if view.size() < output.len() || !playing {
                    output.fill(0);
                } else {
                    let read = ring_buffer.write().unwrap().read_to(output);
                    let volume = volume.load(Ordering::Relaxed) as f32 / 100.0;
                    for s in output.iter_mut() {
                        let val = *s as f32;
                        let out = (val * volume).clamp(i16::MIN as f32, i16::MAX as f32);
                        *s = out as i16
                    }
                    output[read..].fill(0);
                }

                if playing {
                    handle.notify_worker();
                }
            }, move |error| {}, None).unwrap()
        };

        stream.pause().unwrap();

        Ok(AudioDevice {
            host,
            device,
            playing,
            stream,
            volume,
            handle,
            ring_buffer
        })
    }

    pub fn play(&mut self) {
        self.stream.play().unwrap();
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn pause(&mut self) {
        self.stream.pause().unwrap();
        self.playing.store(false, Ordering::Relaxed);
    }
}

impl Drop for AudioDevice {
    fn drop(&mut self) {
        self.ring_buffer.write().unwrap().close();
        self.handle.notify_worker();
        self.stream.pause().unwrap();
    }
}