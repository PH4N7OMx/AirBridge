//! Bounded background MP4 recording. Encoding and disk I/O never block playback.
use crate::media::lock;
use ffmpeg_next as ff;
use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

enum Frame {
    Video(ff::frame::Video, i64),
    Audio(ff::frame::Audio, i64),
}
struct Worker {
    sender: SyncSender<Frame>,
    thread: JoinHandle<()>,
    video_pending: Arc<AtomicUsize>,
}
pub struct Recorder {
    wanted: AtomicBool,
    worker: Mutex<Option<Worker>>,
    retired: Mutex<Vec<JoinHandle<()>>>,
    directory: PathBuf,
    epoch: Mutex<Instant>,
}
impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("active", &self.active())
            .finish()
    }
}
impl Recorder {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            wanted: AtomicBool::new(false),
            worker: Mutex::new(None),
            retired: Mutex::new(Vec::new()),
            directory,
            epoch: Mutex::new(Instant::now()),
        }
    }
    pub fn active(&self) -> bool {
        self.wanted.load(Ordering::Acquire)
    }
    pub fn toggle(&self) {
        if self.active() {
            self.stop();
        } else {
            *lock(&self.epoch) = Instant::now();
            self.wanted.store(true, Ordering::Release);
            log::info!("recording requested; waiting for video");
        }
    }
    pub fn stop(&self) {
        self.wanted.store(false, Ordering::Release);
        if let Some(worker) = lock(&self.worker).take() {
            drop(worker.sender);
            lock(&self.retired).push(worker.thread);
        }
    }
    pub fn finish(&self) {
        self.stop();
        for handle in lock(&self.retired).drain(..) {
            let _ = handle.join();
        }
    }
    pub fn video(&self, frame: &ff::frame::Video) {
        if !self.active() {
            return;
        }
        let mut worker = lock(&self.worker);
        if worker.as_ref().is_some_and(|w| w.thread.is_finished()) {
            self.wanted.store(false, Ordering::Release);
            if let Some(w) = worker.take() {
                let _ = w.thread.join();
            }
            return;
        }
        if worker.is_none() {
            if let Err(e) = std::fs::create_dir_all(&self.directory) {
                log::error!("recording directory: {e}");
                self.wanted.store(false, Ordering::Release);
                return;
            }
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let path = self.directory.join(format!("AirBridge-{stamp}.mp4"));
            let (tx, rx) = mpsc::sync_channel(64);
            let video_pending = Arc::new(AtomicUsize::new(0));
            let pending = video_pending.clone();
            let (w, h) = (frame.width(), frame.height());
            let handle = thread::spawn(move || {
                let result = (|| -> Result<(), ff::Error> {
                    let mut mux = Muxer::new(&path, w, h)?;
                    log::info!("recording {}", path.display());
                    for frame in rx {
                        match frame {
                            Frame::Video(frame, ts) => {
                                pending.fetch_sub(1, Ordering::Relaxed);
                                mux.video(frame, ts)?;
                            }
                            Frame::Audio(frame, ts) => mux.audio(frame, ts)?,
                        }
                    }
                    mux.finish()?;
                    log::info!("recording finalized {}", path.display());
                    Ok(())
                })();
                if let Err(e) = result {
                    log::error!("recording failed {}: {e}", path.display());
                }
            });
            *lock(&self.epoch) = Instant::now();
            *worker = Some(Worker {
                sender: tx,
                thread: handle,
                video_pending,
            });
        }
        let timestamp = lock(&self.epoch)
            .elapsed()
            .as_micros()
            .min(i64::MAX as u128) as i64;
        if let Some(worker) = worker.as_ref()
            && worker.video_pending.load(Ordering::Relaxed) < 2
        {
            worker.video_pending.fetch_add(1, Ordering::Relaxed);
            if worker
                .sender
                .try_send(Frame::Video(frame.clone(), timestamp))
                .is_err()
            {
                worker.video_pending.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
    pub fn audio(&self, frame: &ff::frame::Audio) {
        if !self.active() {
            return;
        }
        let timestamp = lock(&self.epoch)
            .elapsed()
            .as_micros()
            .min(i64::MAX as u128) as i64;
        if let Some(worker) = lock(&self.worker).as_ref() {
            let _ = worker
                .sender
                .try_send(Frame::Audio(frame.clone(), timestamp));
        }
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.finish();
    }
}

struct Muxer {
    output: ff::format::context::Output,
    video: ff::encoder::Video,
    audio: ff::encoder::Audio,
    scaler: Option<ff::software::scaling::Context>,
    resampler: Option<ff::software::resampling::Context>,
    left: std::collections::VecDeque<f32>,
    right: std::collections::VecDeque<f32>,
    audio_pts: i64,
    video_pts: i64,
    finalized: bool,
}
impl Muxer {
    fn new(path: &std::path::Path, width: u32, height: u32) -> Result<Self, ff::Error> {
        let mut output = ff::format::output(path)?;
        let codec = ff::encoder::find_by_name("libx264").ok_or(ff::Error::EncoderNotFound)?;
        let mut video = ff::codec::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        video.set_width(width & !1);
        video.set_height(height & !1);
        video.set_format(ff::format::Pixel::YUV420P);
        video.set_time_base((1, 90000));
        video.set_frame_rate(Some((60, 1)));
        video.set_bit_rate(8_000_000);
        video.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        video.set_gop(120);
        video.set_max_b_frames(0);
        let mut options = ff::Dictionary::new();
        options.set("preset", "ultrafast");
        options.set("tune", "zerolatency");
        let video = video.open_with(options)?;
        {
            let mut stream = output.add_stream(codec)?;
            stream.set_time_base((1, 90000));
            stream.set_parameters(&video);
        }
        let codec = ff::encoder::find(ff::codec::Id::AAC).ok_or(ff::Error::EncoderNotFound)?;
        let mut audio = ff::codec::Context::new_with_codec(codec)
            .encoder()
            .audio()?;
        audio.set_rate(48000);
        audio.set_channel_layout(ff::ChannelLayout::STEREO);
        audio.set_format(ff::format::Sample::F32(ff::format::sample::Type::Planar));
        audio.set_time_base((1, 48000));
        audio.set_bit_rate(192000);
        audio.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        let audio = audio.open_as(codec)?;
        {
            let mut stream = output.add_stream(codec)?;
            stream.set_time_base((1, 48000));
            stream.set_parameters(&audio);
        }
        // Fragments remain inspectable after a crash instead of losing the entire moov atom.
        let mut opts = ff::Dictionary::new();
        opts.set("movflags", "frag_keyframe+empty_moov+default_base_moof");
        output.write_header_with(opts)?;
        Ok(Self {
            output,
            video,
            audio,
            scaler: None,
            resampler: None,
            left: Default::default(),
            right: Default::default(),
            audio_pts: -1,
            video_pts: -1,
            finalized: false,
        })
    }
    fn video(&mut self, frame: ff::frame::Video, ts: i64) -> Result<(), ff::Error> {
        let changed = self.scaler.as_ref().is_none_or(|s| {
            s.input().width != frame.width()
                || s.input().height != frame.height()
                || s.input().format != frame.format()
        });
        if changed {
            self.scaler = Some(ff::software::scaling::Context::get(
                frame.format(),
                frame.width(),
                frame.height(),
                self.video.format(),
                self.video.width(),
                self.video.height(),
                ff::software::scaling::Flags::FAST_BILINEAR,
            )?);
        }
        let mut yuv = ff::frame::Video::empty();
        self.scaler.as_mut().unwrap().run(&frame, &mut yuv)?;
        self.video_pts = (ts * 9 / 100).max(self.video_pts + 1);
        yuv.set_pts(Some(self.video_pts));
        yuv.set_kind(ff::picture::Type::None);
        self.video.send_frame(&yuv)?;
        self.drain_video()
    }
    fn audio(&mut self, frame: ff::frame::Audio, ts: i64) -> Result<(), ff::Error> {
        let changed = self.resampler.as_ref().is_none_or(|r| {
            r.input().format != frame.format()
                || r.input().rate != frame.rate()
                || r.input().channel_layout != frame.channel_layout()
        });
        if changed {
            self.resampler = Some(ff::software::resampling::Context::get(
                frame.format(),
                frame.channel_layout(),
                frame.rate(),
                self.audio.format(),
                ff::ChannelLayout::STEREO,
                48000,
            )?);
        }
        let mut planar = ff::frame::Audio::empty();
        self.resampler.as_mut().unwrap().run(&frame, &mut planar)?;
        if self.audio_pts < 0 {
            self.audio_pts = ts * 48 / 1000;
        }
        self.left.extend(planar.plane::<f32>(0));
        self.right.extend(planar.plane::<f32>(1));
        let samples = self.audio.frame_size() as usize;
        while self.left.len() >= samples {
            let mut frame =
                ff::frame::Audio::new(self.audio.format(), samples, ff::ChannelLayout::STEREO);
            frame.set_rate(48000);
            frame.set_pts(Some(self.audio_pts));
            self.audio_pts += samples as i64;
            for (dst, src) in frame
                .plane_mut::<f32>(0)
                .iter_mut()
                .zip(self.left.drain(..samples))
            {
                *dst = src;
            }
            for (dst, src) in frame
                .plane_mut::<f32>(1)
                .iter_mut()
                .zip(self.right.drain(..samples))
            {
                *dst = src;
            }
            self.audio.send_frame(&frame)?;
            self.drain_audio()?;
        }
        Ok(())
    }
    fn drain_video(&mut self) -> Result<(), ff::Error> {
        let mut packet = ff::Packet::empty();
        loop {
            match self.video.receive_packet(&mut packet) {
                Ok(()) => {}
                Err(ff::Error::Other { errno: 11 }) | Err(ff::Error::Eof) => break,
                Err(e) => return Err(e),
            }
            packet.set_stream(0);
            packet.rescale_ts((1, 90000), self.output.stream(0).unwrap().time_base());
            packet.write_interleaved(&mut self.output)?;
        }
        Ok(())
    }
    fn drain_audio(&mut self) -> Result<(), ff::Error> {
        let mut packet = ff::Packet::empty();
        loop {
            match self.audio.receive_packet(&mut packet) {
                Ok(()) => {}
                Err(ff::Error::Other { errno: 11 }) | Err(ff::Error::Eof) => break,
                Err(e) => return Err(e),
            }
            packet.set_stream(1);
            packet.rescale_ts((1, 48000), self.output.stream(1).unwrap().time_base());
            packet.write_interleaved(&mut self.output)?;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<(), ff::Error> {
        if self.finalized {
            return Ok(());
        }
        self.video.send_eof()?;
        self.drain_video()?;
        self.audio.send_eof()?;
        self.drain_audio()?;
        self.finalized = true;
        self.output.write_trailer()
    }
}
impl Drop for Muxer {
    fn drop(&mut self) {
        if !self.finalized {
            let _ = self.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp4_has_decodable_video_and_aac_audio() {
        ff::init().unwrap();
        let path =
            std::env::temp_dir().join(format!("airbridge-mux-test-{}.mp4", std::process::id()));
        let mut mux = Muxer::new(&path, 64, 96).unwrap();
        for i in 0..30 {
            let mut video = ff::frame::Video::new(ff::format::Pixel::YUV420P, 64, 96);
            video.data_mut(0).fill((i * 7) as u8);
            video.data_mut(1).fill(128);
            video.data_mut(2).fill(128);
            mux.video(video, i * 1_000_000 / 30).unwrap();
            let mut audio = ff::frame::Audio::new(
                ff::format::Sample::F32(ff::format::sample::Type::Planar),
                1600,
                ff::ChannelLayout::STEREO,
            );
            audio.set_rate(48000);
            audio.plane_mut::<f32>(0).fill(0.0);
            audio.plane_mut::<f32>(1).fill(0.0);
            mux.audio(audio, i * 1_000_000 / 30).unwrap();
        }
        mux.finish().unwrap();
        drop(mux);
        let mut input = ff::format::input(&path).unwrap();
        let v = input.streams().best(ff::media::Type::Video).unwrap();
        let mut decoder = ff::codec::Context::from_parameters(v.parameters())
            .unwrap()
            .decoder()
            .video()
            .unwrap();
        let audio = input.streams().best(ff::media::Type::Audio).unwrap();
        assert_eq!(audio.parameters().id(), ff::codec::Id::AAC);
        let mut audio_decoder = ff::codec::Context::from_parameters(audio.parameters())
            .unwrap()
            .decoder()
            .audio()
            .unwrap();
        let mut pictures = 0;
        let mut samples = 0;
        for (stream, packet) in input.packets() {
            if stream.index() == 0 {
                decoder.send_packet(&packet).unwrap();
                let mut frame = ff::frame::Video::empty();
                while decoder.receive_frame(&mut frame).is_ok() {
                    pictures += 1;
                    assert_eq!((frame.width(), frame.height()), (64, 96));
                }
            } else {
                audio_decoder.send_packet(&packet).unwrap();
                let mut frame = ff::frame::Audio::empty();
                while audio_decoder.receive_frame(&mut frame).is_ok() {
                    samples += frame.samples();
                }
            }
        }
        assert_eq!(pictures, 30);
        assert!(samples >= 46000, "decoded {samples} audio samples");
        drop(input);
        std::fs::remove_file(path).unwrap();
    }
}
