use std::sync::{Arc, RwLock};
use std::sync::mpsc::Sender;
use cpal::{BufferSize, ChannelCount, Device, Host, SampleFormat, Stream, StreamConfig};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ffmpeg_sys_next::{av_ripemd_init, AVSampleFormat};
use crate::ffmpeg::frame::SampleType;
use crate::player::decoder::{AudioRingBuffer, DecodeWorkerMessage};

pub struct AudioDevice {
    host: Host,
    device: Device,
    stream: Stream,
    pub ring_buffer: Arc<RwLock<AudioRingBuffer>>,
}

impl AudioDevice {

    pub fn default_device(ring_buffer: Arc<RwLock<AudioRingBuffer>>, sender: Sender<DecodeWorkerMessage>) -> Result<AudioDevice, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().unwrap();

        let config = {
            let mut buffer = ring_buffer.write().unwrap();
            let channels = buffer.channels;
            let sample_rate = buffer.sample_rate;
            let buffer_size = 128 * channels as u32;
            buffer.latency = Some(- (buffer_size as f64 / sample_rate as f64));
            StreamConfig {
                channels,
                sample_rate,
                buffer_size: BufferSize::Fixed(buffer_size),
            }
        };

        let stream = {
            let ring_buffer = ring_buffer.clone();
            device.build_output_stream(&config, move |output: &mut [i16], info: &cpal::OutputCallbackInfo| {
                if ring_buffer.read().unwrap().available() < output.len() {
                    output.fill(0);
                } else {
                    ring_buffer.write().unwrap().read(output);
                    sender.send(DecodeWorkerMessage::Wakeup).unwrap();
                }
            }, move |error| {}, None).unwrap()
        };

        Ok(AudioDevice {
            host,
            device,
            stream,
            ring_buffer
        })
    }

    pub fn play(&mut self) {
        self.stream.play().unwrap();
    }
}