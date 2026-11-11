//! Presentation skips stale decoded images, never compressed reference pictures.
use crate::media::{Model, Picture, lock};
use ffmpeg_next as ff;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

#[derive(Default)]
struct Pending {
    frame: Option<ff::frame::Video>,
    closed: bool,
    epoch: u64,
    decoded: u64,
    skipped: u64,
}
struct Mailbox {
    state: Mutex<Pending>,
    ready: Condvar,
}
pub struct Display {
    mailbox: Arc<Mailbox>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Display {
    pub fn new(model: Arc<Mutex<Model>>, generation: u64) -> Self {
        let mailbox = Arc::new(Mailbox {
            state: Mutex::new(Pending::default()),
            ready: Condvar::new(),
        });
        let input = mailbox.clone();
        let worker = std::thread::spawn(move || {
            let mut scaler: Option<ff::software::scaling::Context> = None;
            let mut last_smooth = None;
            let mut displayed = 0u64;
            loop {
                let (mut decoded, epoch) = {
                    let mut slot = lock(&input.state);
                    while slot.frame.is_none() && !slot.closed {
                        slot = input.ready.wait(slot).unwrap_or_else(|p| p.into_inner());
                    }
                    if slot.closed {
                        return;
                    }
                    (slot.frame.take().unwrap(), slot.epoch)
                };
                let result = (|| -> Result<Picture, ff::Error> {
                    if decoded.format() == ff::format::Pixel::D3D11 {
                        let mut downloaded = ff::frame::Video::empty();
                        let result = unsafe {
                            ff::ffi::av_hwframe_transfer_data(
                                downloaded.as_mut_ptr(),
                                decoded.as_ptr(),
                                0,
                            )
                        };
                        if result < 0 {
                            return Err(ff::Error::from(result));
                        }
                        decoded = downloaded;
                    }
                    let recorder = lock(&model).recorder.clone();
                    if let Some(recorder) = recorder {
                        recorder.video(&decoded);
                    }
                    let (width, height) = (decoded.width(), decoded.height());
                    let (bounds, smooth, unfiltered) = {
                        let m = lock(&model);
                        (m.display_size, m.smooth_scaling, m.unfiltered_scaling)
                    };
                    let (out_width, out_height) = presentation_size(width, height, bounds);
                    if scaler.as_ref().is_none_or(|s| {
                        s.input().width != width
                            || s.input().height != height
                            || s.input().format != decoded.format()
                            || s.output().width != out_width
                            || s.output().height != out_height
                            || last_smooth != Some((smooth, unfiltered))
                    }) {
                        scaler = Some(ff::software::scaling::Context::get(
                            decoded.format(),
                            width,
                            height,
                            ff::format::Pixel::BGRA,
                            out_width,
                            out_height,
                            // Both filters suppress downscale aliasing. Lanczos
                            // retains more edge contrast for small phone text;
                            // area averaging is available for a softer picture.
                            (if unfiltered {
                                ff::software::scaling::Flags::POINT
                            } else if smooth {
                                ff::software::scaling::Flags::AREA
                            } else {
                                ff::software::scaling::Flags::LANCZOS
                            }) | ff::software::scaling::Flags::ACCURATE_RND,
                        )?);
                        last_smooth = Some((smooth, unfiltered));
                        log::info!(
                            "display video format {width}x{height} -> {out_width}x{out_height} scaling={}",
                            if unfiltered {
                                "unfiltered-nearest"
                            } else if smooth {
                                "smooth"
                            } else {
                                "sharp"
                            }
                        );
                    }
                    let mut bgra = ff::frame::Video::empty();
                    scaler.as_mut().unwrap().run(&decoded, &mut bgra)?;
                    let row = out_width as usize * 4;
                    let mut pixels = Vec::with_capacity(row * out_height as usize);
                    for line in bgra
                        .data(0)
                        .chunks(bgra.stride(0))
                        .take(out_height as usize)
                    {
                        pixels.extend_from_slice(&line[..row]);
                    }
                    Ok(Picture {
                        width: out_width,
                        height: out_height,
                        pixels,
                    })
                })();
                match result {
                    Ok(picture) => {
                        let slot = lock(&input.state);
                        if slot.closed || slot.epoch != epoch {
                            continue;
                        }
                        let mut m = lock(&model);
                        if m.generation != generation {
                            return;
                        }
                        displayed += 1;
                        m.frames += 1;
                        m.picture = Some(Arc::new(picture));
                        crate::ui::notify_video_frame();
                        if displayed == 1 || displayed.is_multiple_of(120) {
                            log::info!(
                                "video progress generation={generation} decoded={} displayed={displayed} skipped_display={}",
                                slot.decoded,
                                slot.skipped
                            );
                        }
                    }
                    Err(e) => log::warn!("display conversion failed: {e}"),
                }
            }
        });
        Self {
            mailbox,
            worker: Mutex::new(Some(worker)),
        }
    }
    pub fn push(&self, frame: ff::frame::Video) {
        let mut slot = lock(&self.mailbox.state);
        if slot.closed {
            return;
        }
        slot.decoded += 1;
        if slot.frame.replace(frame).is_some() {
            slot.skipped += 1;
        }
        self.mailbox.ready.notify_one();
    }
    pub fn flush(&self) {
        let mut slot = lock(&self.mailbox.state);
        slot.frame = None;
        slot.epoch += 1;
    }
    pub fn close(&self) {
        {
            let mut slot = lock(&self.mailbox.state);
            slot.closed = true;
            slot.frame = None;
            self.mailbox.ready.notify_all();
        }
        if let Some(worker) = lock(&self.worker).take() {
            let _ = worker.join();
        }
    }
}

fn presentation_size(width: u32, height: u32, bounds: (u32, u32)) -> (u32, u32) {
    if bounds.0 == 0 || bounds.1 == 0 {
        return (width, height);
    }
    let scale = (bounds.0 as f64 / width as f64)
        .min(bounds.1 as f64 / height as f64)
        .min(1.0);
    (
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
    )
}

impl Drop for Display {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaling_choice_updates_live_without_changing_stream_dimensions() {
        ff::init().unwrap();
        let model = Arc::new(Mutex::new(Model {
            generation: 1,
            display_size: (43, 57),
            ..Default::default()
        }));
        let display = Display::new(model.clone(), 1);
        let mut images = Vec::new();
        for (smooth, unfiltered) in [(false, false), (true, false), (false, false), (false, true)] {
            {
                let mut m = lock(&model);
                m.smooth_scaling = smooth;
                m.unfiltered_scaling = unfiltered;
                m.picture = None;
            }
            let mut frame = ff::frame::Video::new(ff::format::Pixel::YUV420P, 96, 128);
            let stride = frame.stride(0);
            for row in frame.data_mut(0).chunks_mut(stride).take(128) {
                for (x, value) in row.iter_mut().take(96).enumerate() {
                    *value = if (x / 3) % 2 == 0 { 16 } else { 235 };
                }
            }
            frame.data_mut(1).fill(128);
            frame.data_mut(2).fill(128);
            display.push(frame);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while lock(&model).picture.is_none() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            images.push(lock(&model).picture.clone().unwrap());
        }
        assert_ne!(
            images[0].pixels, images[1].pixels,
            "live filter did not change"
        );
        assert_eq!(
            images[0].pixels, images[2].pixels,
            "sharp filter was not restored"
        );
        assert_eq!((images[0].width, images[0].height), (43, 57));
        assert_ne!(
            images[0].pixels, images[3].pixels,
            "unfiltered mode did not change sampling"
        );
        display.close();
    }
    #[test]
    fn phone_fine_details_downscale_without_pixel_aliasing() {
        ff::init().unwrap();
        let model = Arc::new(Mutex::new(Model {
            generation: 1,
            display_size: (434, 943),
            ..Default::default()
        }));
        let display = Display::new(model.clone(), 1);
        let mut frame = ff::frame::Video::new(ff::format::Pixel::YUV420P, 1884, 4096);
        let stride = frame.stride(0);
        for row in frame.data_mut(0).chunks_mut(stride).take(4096) {
            for (x, value) in row.iter_mut().take(1884).enumerate() {
                *value = if x % 2 == 0 { 16 } else { 235 };
            }
        }
        frame.data_mut(1).fill(128);
        frame.data_mut(2).fill(128);
        display.push(frame);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while lock(&model).picture.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let picture = lock(&model).picture.clone().unwrap();
        let row = &picture.pixels[(picture.height / 2 * picture.width * 4) as usize..];
        let values: Vec<_> = row
            .as_chunks::<4>()
            .0
            .iter()
            .take(picture.width as usize - 10)
            .skip(10)
            .map(|p| p[0])
            .collect();
        let spread = *values.iter().max().unwrap() as i32 - *values.iter().min().unwrap() as i32;
        assert!(
            spread < 64,
            "fine details aliased into visible blocks: brightness spread {spread}"
        );
        display.close();
    }
    #[test]
    fn conversion_matches_window_and_never_upscales() {
        assert_eq!(presentation_size(1884, 4096, (500, 1000)), (460, 1000));
        assert_eq!(presentation_size(1884, 4096, (3840, 2160)), (994, 2160));
        assert_eq!(presentation_size(64, 96, (500, 1000)), (64, 96));
        assert_eq!(presentation_size(1884, 4096, (0, 0)), (1884, 4096));
    }
    #[test]
    fn large_video_conversion_tracks_window_resize() {
        ff::init().unwrap();
        let model = Arc::new(Mutex::new(Model {
            generation: 1,
            display_size: (500, 1000),
            ..Default::default()
        }));
        let display = Display::new(model.clone(), 1);
        for (size, expected) in [((500, 1000), (460, 1000)), ((3840, 2160), (994, 2160))] {
            {
                let mut m = lock(&model);
                m.display_size = size;
                m.picture = None;
            }
            let mut frame = ff::frame::Video::new(ff::format::Pixel::YUV420P, 1884, 4096);
            for plane in 0..3 {
                frame.data_mut(plane).fill(128);
            }
            display.push(frame);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while lock(&model).picture.is_none() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let picture = lock(&model).picture.clone().unwrap();
            assert_eq!((picture.width, picture.height), expected);
            assert_eq!(picture.pixels.len(), (expected.0 * expected.1 * 4) as usize);
        }
        display.close();
    }
}
