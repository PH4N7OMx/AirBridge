use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ffmpeg_next as ff;
use rust_airplay::{
    AudioCodec, AudioConfig, AudioFrame, AudioStream, RenderError, Renderer, VideoCodec,
    VideoConfig, VideoFrame, VideoStream, Volume,
};
use std::{
    collections::VecDeque,
    fmt,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn fail(e: impl fmt::Display) -> RenderError {
    RenderError::Failed(e.to_string())
}
#[derive(Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
#[derive(Debug, Default)]
pub struct Model {
    pub picture: Option<Arc<Picture>>,
    pub artwork: Option<Arc<Picture>>,
    pub frames: u64,
    pub decoded_frames: u64,
    pub audio_frames: u64,
    pub status: String,
    pub metadata: String,
    pub generation: u64,
    pub rotation: u32,
    pub display_size: (u32, u32),
    pub smooth_scaling: bool,
    pub unfiltered_scaling: bool,
    pub recorder: Option<Arc<crate::recording::Recorder>>,
}
#[derive(Debug)]
pub struct Playback {
    pub model: Arc<Mutex<Model>>,
    gain: Arc<AtomicU32>,
    video_generation: AtomicU64,
    audio_generation: Arc<AtomicU64>,
    pub hardware: bool,
}
impl Playback {
    pub fn new(model: Arc<Mutex<Model>>) -> Self {
        Self {
            model,
            gain: Arc::new(AtomicU32::new(1f32.to_bits())),
            video_generation: AtomicU64::new(0),
            audio_generation: Arc::new(AtomicU64::new(0)),
            hardware: true,
        }
    }
}
impl Renderer for Playback {
    fn open_video(&self, cfg: VideoConfig) -> Result<Box<dyn VideoStream>, RenderError> {
        let id = match cfg.codec {
            VideoCodec::H264 => ff::codec::Id::H264,
            VideoCodec::H265 => ff::codec::Id::HEVC,
            _ => return Err(fail("Unsupported video codec")),
        };
        let codec = ff::decoder::find(id).ok_or_else(|| fail("Video decoder missing"))?;
        let mut context = ff::codec::Context::new_with_codec(codec);
        unsafe {
            (*context.as_mut_ptr()).max_pixels = 32 * 1024 * 1024;
        }
        context.set_threading(ff::codec::threading::Config {
            kind: ff::codec::threading::Type::Slice,
            count: 2,
        });
        if self.hardware {
            unsafe {
                let mut device = std::ptr::null_mut();
                let result = ff::ffi::av_hwdevice_ctx_create(
                    &mut device,
                    ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    0,
                );
                if result >= 0 {
                    (*context.as_mut_ptr()).hw_device_ctx = device;
                    (*context.as_mut_ptr()).get_format = Some(hardware_format);
                    log::info!("D3D11VA hardware decoding available");
                } else {
                    log::warn!("D3D11VA unavailable; using software decoder");
                }
            }
        }
        let decoder = context.decoder().video().map_err(fail)?;
        let generation = self.video_generation.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let mut m = lock(&self.model);
            m.generation = generation;
            m.picture = None;
            m.status = format!("Receiving {} video", cfg.codec.name());
        }
        log::info!(
            "video opened generation={generation} codec={}",
            cfg.codec.name()
        );
        Ok(Box::new(Video {
            state: Mutex::new(VideoState {
                decoder,
                closed: false,
            }),
            display: crate::video_display::Display::new(self.model.clone(), generation),
            model: self.model.clone(),
            generation,
        }))
    }
    fn open_audio(&self, cfg: AudioConfig) -> Result<Box<dyn AudioStream>, RenderError> {
        if !(1..=2).contains(&cfg.channels) || !(8000..=96000).contains(&cfg.sample_rate) {
            return Err(fail("Invalid audio format"));
        }
        let (id, extra) = match cfg.codec {
            AudioCodec::AacEld => (
                ff::codec::Id::AAC,
                aac_eld_config(cfg.sample_rate, cfg.channels, cfg.samples_per_frame)?,
            ),
            AudioCodec::Alac => (
                ff::codec::Id::ALAC,
                alac_config(cfg.sample_rate, cfg.channels, cfg.samples_per_frame),
            ),
            _ => return Err(fail("Unsupported audio codec")),
        };
        let codec = ff::decoder::find(id).ok_or_else(|| fail("Audio decoder missing"))?;
        let mut ctx = ff::codec::Context::new_with_codec(codec);
        unsafe {
            let p = ctx.as_mut_ptr();
            (*p).sample_rate = cfg.sample_rate as i32;
            ff::ffi::av_channel_layout_default(&mut (*p).ch_layout, cfg.channels.into());
            let buffer =
                ff::ffi::av_mallocz(extra.len() + ff::ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize)
                    as *mut u8;
            if buffer.is_null() {
                return Err(fail("Out of memory allocating audio configuration"));
            }
            std::ptr::copy_nonoverlapping(extra.as_ptr(), buffer, extra.len());
            (*p).extradata = buffer;
            (*p).extradata_size = extra.len() as i32;
        }
        let decoder = ctx.decoder().audio().map_err(fail)?;
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| fail("No audio output device"))?;
        let supported = device.default_output_config().map_err(fail)?;
        let output: cpal::StreamConfig = supported.clone().into();
        if output.channels > 8 {
            return Err(fail("Audio output has too many channels"));
        }
        let generation = self.audio_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let queue = Arc::new(Mutex::new(AudioQueue::new(
            output.sample_rate,
            output.channels,
        )));
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => audio_device::<f32>(
                &device,
                &output,
                queue.clone(),
                self.gain.clone(),
                self.audio_generation.clone(),
                generation,
            ),
            cpal::SampleFormat::I16 => audio_device::<i16>(
                &device,
                &output,
                queue.clone(),
                self.gain.clone(),
                self.audio_generation.clone(),
                generation,
            ),
            cpal::SampleFormat::U16 => audio_device::<u16>(
                &device,
                &output,
                queue.clone(),
                self.gain.clone(),
                self.audio_generation.clone(),
                generation,
            ),
            other => return Err(fail(format!("Unsupported output format: {other}"))),
        }?;
        stream.play().map_err(fail)?;
        let rate = output.sample_rate;
        log::info!(
            "audio opened {:?} {}Hz {}ch, output {}Hz {}ch generation={generation}",
            cfg.codec,
            cfg.sample_rate,
            cfg.channels,
            rate,
            output.channels
        );
        Ok(Box::new(Audio {
            state: Mutex::new(AudioState {
                decoder,
                resampler: None,
                queue,
                _device: stream,
                output,
                closed: false,
                frames: 0,
            }),
            model: self.model.clone(),
            active_generation: self.audio_generation.clone(),
            generation,
        }))
    }
    fn set_volume(&self, volume: Volume) -> Result<(), RenderError> {
        self.gain
            .store((volume.level() as f32).to_bits(), Ordering::Relaxed);
        Ok(())
    }
}

struct VideoState {
    decoder: ff::decoder::Video,
    closed: bool,
}
struct Video {
    state: Mutex<VideoState>,
    display: crate::video_display::Display,
    model: Arc<Mutex<Model>>,
    generation: u64,
}
impl fmt::Debug for Video {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VideoStream")
    }
}
impl Video {
    fn drain(&self, state: &mut VideoState) -> Result<(), RenderError> {
        let mut decoded = ff::frame::Video::empty();
        loop {
            match state.decoder.receive_frame(&mut decoded) {
                Ok(()) => {}
                Err(ff::Error::Other { errno: 11 }) | Err(ff::Error::Eof) => break,
                Err(e) => return Err(fail(e)),
            }
            let w = decoded.width();
            let h = decoded.height();
            if w == 0
                || h == 0
                || w > 8192
                || h > 8192
                || u64::from(w) * u64::from(h) > 32 * 1024 * 1024
            {
                return Err(fail("Video dimensions exceed safe limit"));
            }
            lock(&self.model).decoded_frames += 1;
            self.display.push(decoded);
            decoded = ff::frame::Video::empty();
        }
        Ok(())
    }
}
unsafe extern "C" fn hardware_format(
    _ctx: *mut ff::ffi::AVCodecContext,
    formats: *const ff::ffi::AVPixelFormat,
) -> ff::ffi::AVPixelFormat {
    unsafe {
        let mut p = formats;
        while *p != ff::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            if *p == ff::ffi::AVPixelFormat::AV_PIX_FMT_D3D11 {
                return *p;
            }
            p = p.add(1);
        }
        ff::ffi::avcodec_default_get_format(_ctx, formats)
    }
}
impl VideoStream for Video {
    fn push(&self, frame: VideoFrame) -> Result<(), RenderError> {
        if lock(&self.model).generation != self.generation {
            return Ok(());
        }
        if frame.data.len() > 16 * 1024 * 1024 {
            return Err(fail("Oversized video packet"));
        }
        let mut s = lock(&self.state);
        if s.closed {
            return Ok(());
        }
        let packet = ff::Packet::copy(&frame.data); // FFmpeg-owned padded copy; never borrow unpadded network bytes.
        match s.decoder.send_packet(&packet) {
            Err(ff::Error::Other { errno: 11 }) => {
                self.drain(&mut s)?;
                s.decoder.send_packet(&packet).map_err(fail)?;
            }
            Err(e) => return Err(fail(e)),
            Ok(()) => {}
        }
        self.drain(&mut s)
    }
    fn flush(&self) -> Result<(), RenderError> {
        let mut s = lock(&self.state);
        s.decoder.flush();
        self.display.flush();
        Ok(())
    }
    fn close(&self) -> Result<(), RenderError> {
        lock(&self.state).closed = true;
        self.display.close();
        let mut m = lock(&self.model);
        if m.generation == self.generation {
            m.picture = None;
            m.status = "Ready — select AirBridge in Screen Mirroring".into();
        }
        Ok(())
    }
}
impl Drop for Video {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

struct AudioState {
    decoder: ff::decoder::Audio,
    resampler: Option<ff::software::resampling::Context>,
    queue: Arc<Mutex<AudioQueue>>,
    _device: cpal::Stream,
    output: cpal::StreamConfig,
    closed: bool,
    frames: u64,
}
struct Audio {
    state: Mutex<AudioState>,
    model: Arc<Mutex<Model>>,
    active_generation: Arc<AtomicU64>,
    generation: u64,
}
impl fmt::Debug for Audio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AudioStream")
    }
}
impl Audio {
    fn drain(&self, state: &mut AudioState) -> Result<(), RenderError> {
        let mut decoded = ff::frame::Audio::empty();
        loop {
            match state.decoder.receive_frame(&mut decoded) {
                Ok(()) => {}
                Err(ff::Error::Other { errno: 11 }) | Err(ff::Error::Eof) => break,
                Err(e) => return Err(fail(e)),
            }
            let layout = decoded.channel_layout();
            let rate = decoded.rate();
            let format = decoded.format();
            if let Some(recorder) = lock(&self.model).recorder.clone() {
                recorder.audio(&decoded);
            }
            let changed = state.resampler.as_ref().is_none_or(|s| {
                s.input().format != format
                    || s.input().channel_layout != layout
                    || s.input().rate != rate
            });
            if changed {
                state.resampler = Some(
                    ff::software::resampling::Context::get(
                        format,
                        layout,
                        rate,
                        ff::format::Sample::F32(ff::format::sample::Type::Packed),
                        ff::ChannelLayout::default(state.output.channels as i32),
                        state.output.sample_rate,
                    )
                    .map_err(fail)?,
                );
            }
            let pcm = resample_audio(state.resampler.as_mut().unwrap(), &decoded).map_err(fail)?;
            let samples = pcm.samples() * state.output.channels as usize;
            let bytes = &pcm.data(0)[..samples * 4];
            let values = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_ne_bytes(*b));
            {
                let mut queue = lock(&state.queue);
                queue.push(values);
                state.frames += 1;
                if state.frames == 1 || state.frames.is_multiple_of(1000) {
                    let delay = state
                        .resampler
                        .as_ref()
                        .and_then(|s| s.delay())
                        .map_or(0, |d| d.output);
                    log::info!(
                        "audio progress decoded={} buffered_ms={} resampler_samples={delay} underruns={} skipped_samples={}",
                        state.frames,
                        queue.samples.len() * 1000
                            / state.output.channels as usize
                            / state.output.sample_rate as usize,
                        queue.underruns,
                        queue.skipped
                    );
                }
            }
            lock(&self.model).audio_frames += 1;
        }
        Ok(())
    }
}
impl AudioStream for Audio {
    fn push(&self, frame: AudioFrame) -> Result<(), RenderError> {
        if frame.data.len() > 1024 * 1024 {
            return Err(fail("Oversized audio packet"));
        }
        if self.active_generation.load(Ordering::Acquire) != self.generation {
            return Ok(());
        }
        let mut s = lock(&self.state);
        if s.closed {
            return Ok(());
        }
        let packet = ff::Packet::copy(&frame.data);
        match s.decoder.send_packet(&packet) {
            Err(ff::Error::Other { errno: 11 }) => {
                self.drain(&mut s)?;
                s.decoder.send_packet(&packet).map_err(fail)?;
            }
            Err(e) => return Err(fail(e)),
            Ok(()) => {}
        }
        self.drain(&mut s)
    }
    fn flush(&self) -> Result<(), RenderError> {
        let mut s = lock(&self.state);
        s.decoder.flush();
        s.resampler = None;
        lock(&s.queue).clear();
        Ok(())
    }
    fn close(&self) -> Result<(), RenderError> {
        let mut s = lock(&self.state);
        s.closed = true;
        lock(&s.queue).clear();
        s._device.pause().map_err(fail)
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn audio_device<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<Mutex<AudioQueue>>,
    gain: Arc<AtomicU32>,
    active: Arc<AtomicU64>,
    generation: u64,
) -> Result<cpal::Stream, RenderError> {
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                let level = f32::from_bits(gain.load(Ordering::Relaxed));
                let current = active.load(Ordering::Acquire) == generation;
                let mut queue = queue.try_lock().ok();
                if let Some(q) = queue.as_mut() {
                    q.prepare();
                }
                for sample in output {
                    let value = if current {
                        queue.as_mut().map_or(0.0, |q| q.next())
                    } else {
                        0.0
                    };
                    *sample = T::from_sample((value * level).clamp(-1.0, 1.0));
                }
            },
            |e| log::error!("audio output: {e}"),
            None,
        )
        .map_err(fail)
}

// ffmpeg-next's empty-output convenience allocation uses input.samples(). That
// is too small when upsampling (480 samples at 44.1 kHz become ~522 at 48 kHz),
// accumulating unplayed sound inside swresample on every packet.
fn resample_audio(
    resampler: &mut ff::software::resampling::Context,
    decoded: &ff::frame::Audio,
) -> Result<ff::frame::Audio, ff::Error> {
    let capacity =
        unsafe { ff::ffi::swr_get_out_samples(resampler.as_mut_ptr(), decoded.samples() as i32) };
    if capacity < 0 {
        return Err(ff::Error::from(capacity));
    }
    let mut pcm = ff::frame::Audio::new(
        resampler.output().format,
        capacity.max(1) as usize,
        resampler.output().channel_layout,
    );
    resampler.run(decoded, &mut pcm)?;
    Ok(pcm)
}

struct AudioQueue {
    samples: VecDeque<f32>,
    playing: bool,
    preroll: usize,
    limit: usize,
    channels: usize,
    underruns: u64,
    skipped: u64,
}
impl AudioQueue {
    fn new(rate: u32, channels: u16) -> Self {
        let channels = channels as usize;
        Self {
            samples: VecDeque::with_capacity(rate as usize * channels / 5),
            playing: false,
            preroll: (rate as usize * 40 / 1000).max(1) * channels,
            limit: (rate as usize * 120 / 1000).max(1) * channels,
            channels,
            underruns: 0,
            skipped: 0,
        }
    }
    fn push(&mut self, values: impl Iterator<Item = f32>) {
        self.samples.extend(values);
        if self.samples.len() > self.limit {
            // Skip complete interleaved frames, keeping left/right in phase.
            let excess = (self.samples.len() - self.preroll) / self.channels * self.channels;
            self.samples.drain(..excess);
            self.skipped += excess as u64;
        }
    }
    fn prepare(&mut self) {
        if self.samples.len() >= self.preroll {
            self.playing = true;
        }
    }
    fn next(&mut self) -> f32 {
        if !self.playing {
            return 0.0;
        }
        self.samples.pop_front().unwrap_or_else(|| {
            self.playing = false;
            self.underruns += 1;
            0.0
        })
    }
    fn clear(&mut self) {
        self.samples.clear();
        self.playing = false;
    }
}

pub fn aac_eld_config(rate: u32, channels: u8, spf: u16) -> Result<Vec<u8>, RenderError> {
    let index = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ]
    .iter()
    .position(|r| *r == rate)
    .ok_or_else(|| fail("Unsupported AAC sample rate"))?;
    if !(1..=2).contains(&channels) || ![480, 512].contains(&spf) {
        return Err(fail("Unsupported AAC-ELD channel/frame configuration"));
    }
    let mut bytes = vec![0u8; 4];
    let mut bit = 0;
    for (value, count) in [
        (31, 5),
        (7, 6),
        (index as u32, 4),
        (channels as u32, 4),
        (u32::from(spf == 480), 1),
        (0, 8),
    ] {
        for shift in (0..count).rev() {
            bytes[bit / 8] |= (((value >> shift) & 1) as u8) << (7 - bit % 8);
            bit += 1;
        }
    }
    Ok(bytes)
}
fn alac_config(rate: u32, channels: u8, spf: u16) -> Vec<u8> {
    let mut data = vec![0, 0, 0, 36, b'a', b'l', b'a', b'c', 0, 0, 0, 0];
    data.extend_from_slice(&u32::from(if spf == 0 { 352 } else { spf }).to_be_bytes());
    data.extend_from_slice(&[0, 16, 40, 10, 14, channels, 0, 255]);
    data.extend_from_slice(&[0; 8]);
    data.extend_from_slice(&rate.to_be_bytes());
    data
}

pub fn artwork(mime: &str, bytes: &[u8]) -> Result<Picture, RenderError> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(fail("Artwork exceeds 8 MiB"));
    }
    let id = match mime {
        "image/png" => ff::codec::Id::PNG,
        "image/jpeg" => ff::codec::Id::MJPEG,
        _ => return Err(fail("Unsupported artwork type")),
    };
    let codec = ff::decoder::find(id).ok_or_else(|| fail("Artwork decoder missing"))?;
    let mut ctx = ff::codec::Context::new_with_codec(codec);
    unsafe {
        (*ctx.as_mut_ptr()).max_pixels = 16 * 1024 * 1024;
    }
    let mut decoder = ctx.decoder().video().map_err(fail)?;
    decoder
        .send_packet(&ff::Packet::copy(bytes))
        .map_err(fail)?;
    let mut frame = ff::frame::Video::empty();
    decoder.receive_frame(&mut frame).map_err(fail)?;
    if frame.width() == 0 || frame.height() == 0 || frame.width() > 4096 || frame.height() > 4096 {
        return Err(fail("Artwork dimensions exceed safe limit"));
    }
    let (width, height) = (frame.width(), frame.height());
    let mut scaler = ff::software::scaling::Context::get(
        frame.format(),
        width,
        height,
        ff::format::Pixel::BGRA,
        width,
        height,
        ff::software::scaling::Flags::BILINEAR,
    )
    .map_err(fail)?;
    let mut bgra = ff::frame::Video::empty();
    scaler.run(&frame, &mut bgra).map_err(fail)?;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in bgra.data(0).chunks(bgra.stride(0)).take(height as usize) {
        pixels.extend_from_slice(&row[..width as usize * 4]);
    }
    Ok(Picture {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upsampling_does_not_accumulate_seconds_of_audio() {
        ff::init().unwrap();
        let format = ff::format::Sample::F32(ff::format::sample::Type::Packed);
        for (input_rate, output_rate) in [(44100, 48000), (48000, 44100), (44100, 44100)] {
            let mut resampler = ff::software::resampling::Context::get(
                format,
                ff::ChannelLayout::STEREO,
                input_rate,
                format,
                ff::ChannelLayout::STEREO,
                output_rate,
            )
            .unwrap();
            let mut input = ff::frame::Audio::new(format, 480, ff::ChannelLayout::STEREO);
            input.set_rate(input_rate);
            input.data_mut(0).fill(0);
            let mut output_samples = 0;
            for _ in 0..1000 {
                output_samples += resample_audio(&mut resampler, &input).unwrap().samples();
            }
            let expected = 480000u64 * output_rate as u64 / input_rate as u64;
            assert!(
                expected.abs_diff(output_samples as u64) < 64,
                "{input_rate} -> {output_rate}: expected {expected}, got {output_samples}"
            );
            assert!(resampler.delay().is_none_or(|d| d.output < 64));
        }
    }
    #[test]
    fn audio_jitter_preroll_and_overflow_preserve_stereo() {
        let mut queue = AudioQueue::new(48000, 2);
        let stereo = |frames| (0..frames).flat_map(|_| [0.25, -0.25]);
        queue.push(stereo(480));
        queue.prepare();
        assert_eq!(queue.next(), 0.0, "do not start with only 10ms buffered");
        assert_eq!(queue.samples.len(), 960);
        queue.push(stereo(1440));
        queue.prepare();
        for _ in 0..480 {
            assert_eq!(queue.next(), 0.25);
            assert_eq!(queue.next(), -0.25);
        }
        queue.push(stereo(10000));
        assert_eq!(
            queue.samples.len(),
            3840,
            "overflow returns to 40ms latency"
        );
        for _ in 0..1920 {
            assert_eq!(queue.next(), 0.25);
            assert_eq!(queue.next(), -0.25);
        }
        assert_eq!(queue.next(), 0.0);
        assert!(!queue.playing);
        queue.push(stereo(480));
        queue.prepare();
        assert_eq!(queue.next(), 0.0, "underrun must refill before restarting");
        queue.clear();
        assert!(queue.samples.is_empty());
        assert!(!queue.playing);
    }
    #[test]
    fn eld_matches_reference_bitstream() {
        assert_eq!(
            aac_eld_config(44100, 2, 480).unwrap(),
            [0xf8, 0xe8, 0x50, 0x00]
        );
        assert!(aac_eld_config(12345, 2, 480).is_err());
    }
    #[test]
    fn alac_cookie_uses_actual_frame_size_and_full_sample_rate() {
        let c = alac_config(96000, 2, 4096);
        assert_eq!(c.len(), 36);
        assert_eq!(&c[12..16], &4096u32.to_be_bytes());
        assert_eq!(&c[32..36], &96000u32.to_be_bytes());
    }
    #[test]
    fn actual_h264_and_hevc_decode_and_reconnect_without_stale_pictures() {
        ff::init().unwrap();
        let model = Arc::new(Mutex::new(Model::default()));
        let mut renderer = Playback::new(model.clone());
        renderer.hardware = false;
        for (file, codec) in [
            ("test.h264", VideoCodec::H264),
            ("test.hevc", VideoCodec::H265),
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures")
                .join(file);
            let mut input = ff::format::input(&path).unwrap();
            let old = renderer
                .open_video(VideoConfig {
                    width: Some(64),
                    height: Some(96),
                    codec,
                })
                .unwrap();
            let current = renderer
                .open_video(VideoConfig {
                    width: Some(64),
                    height: Some(96),
                    codec,
                })
                .unwrap();
            let before = lock(&model).decoded_frames;
            for (_, packet) in input.packets() {
                current
                    .push(VideoFrame {
                        data: packet.data().unwrap().to_vec(),
                        timestamp: 0,
                        keyframe: packet.is_key(),
                    })
                    .unwrap();
            }
            assert!(lock(&model).decoded_frames >= before + 8);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while lock(&model).picture.is_none() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let p = lock(&model).picture.clone().unwrap();
            assert_eq!((p.width, p.height), (64, 96));
            old.close().unwrap();
            assert!(
                lock(&model).picture.is_some(),
                "stale stream erased new picture"
            );
            current.close().unwrap();
            assert!(lock(&model).picture.is_none());
        }
    }
    #[test]
    fn bundled_codecs_include_modern_d3d11_hardware_backends() {
        ff::init().unwrap();
        for id in [ff::codec::Id::H264, ff::codec::Id::HEVC] {
            let codec = ff::decoder::find(id).expect("required video decoder");
            let mut modern_backend = false;
            for index in 0..64 {
                let config = unsafe { ff::ffi::avcodec_get_hw_config(codec.as_ptr(), index) };
                if config.is_null() {
                    break;
                }
                let config = unsafe { &*config };
                modern_backend |= config.pix_fmt == ff::ffi::AVPixelFormat::AV_PIX_FMT_D3D11
                    && config.device_type
                        == ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA;
            }
            assert!(modern_backend, "{id:?} lacks the D3D11 frame backend");
        }
    }
    #[test]
    fn high_resolution_hevc_burst_keeps_decoding_after_first_frame() {
        ff::init().unwrap();
        let model = Arc::new(Mutex::new(Model::default()));
        let renderer = Playback::new(model.clone());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/burst-4k.hevc");
        let mut input = ff::format::input(&path).unwrap();
        let stream = renderer
            .open_video(VideoConfig {
                width: None,
                height: None,
                codec: VideoCodec::H265,
            })
            .unwrap();
        let mut packets = input.packets();
        let (_, first) = packets.next().unwrap();
        stream
            .push(VideoFrame {
                data: first.data().unwrap().to_vec(),
                timestamp: 0,
                keyframe: first.is_key(),
            })
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while lock(&model).picture.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let first_picture = lock(&model).picture.clone().expect("first frame");
        let mut keyframes = usize::from(first.is_key());
        let mut count = 1;
        for (_, packet) in packets {
            keyframes += usize::from(packet.is_key());
            count += 1;
            stream
                .push(VideoFrame {
                    data: packet.data().unwrap().to_vec(),
                    timestamp: 0,
                    keyframe: packet.is_key(),
                })
                .unwrap();
        }
        assert_eq!(
            keyframes, 1,
            "regression must not recover through a second keyframe"
        );
        assert_eq!(lock(&model).decoded_frames, count);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while lock(&model).frames < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let latest = lock(&model).picture.clone().unwrap();
        assert_eq!((latest.width, latest.height), (1884, 4096));
        assert_ne!(
            latest.pixels, first_picture.pixels,
            "display remained on first picture"
        );
        stream.close().unwrap();
    }
    #[test]
    fn artwork_is_decoded_with_bounds_and_invalid_images_are_rejected() {
        ff::init().unwrap();
        let image = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/cover.png"),
        )
        .unwrap();
        let p = artwork("image/png", &image).unwrap();
        assert_eq!((p.width, p.height, p.pixels.len()), (64, 96, 64 * 96 * 4));
        assert!(artwork("image/png", b"invalid").is_err());
        assert!(artwork("application/octet-stream", &image).is_err());
    }
}
