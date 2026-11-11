#![windows_subsystem = "windows"]
mod i18n;
mod icon;
mod logging;
mod media;
mod platform;
#[allow(dead_code)] // Retained media/codec tests; recording is not exposed by the app.
mod recording;
mod settings;
mod supervisor;
mod theme_transition;
mod ui;
mod ui_style;
mod video_display;
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub static RECEIVER_FAILED: AtomicBool = AtomicBool::new(false);
pub fn root() -> io::Result<PathBuf> {
    if let Some(root) = std::env::var_os("AIRBRIDGE_ROOT") {
        return Ok(root.into());
    }
    let exe = std::env::current_exe()?;
    let parent = exe
        .parent()
        .ok_or_else(|| io::Error::other("Executable has no directory"))?;
    let development_binary = parent
        .file_name()
        .is_some_and(|n| n == "debug" || n == "release")
        && parent
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == "target");
    if !development_binary {
        return Ok(parent.into());
    }
    let checkout = parent
        .ancestors()
        .nth(4)
        .filter(|root| root.join("source/rust/Cargo.toml").is_file());
    Ok(checkout.unwrap_or(parent).into())
}
fn option(name: &str) -> Option<String> {
    let args = std::env::args().collect::<Vec<_>>();
    args.windows(2).find(|p| p[0] == name).map(|p| p[1].clone())
}
fn receiver(root: PathBuf) -> Result<i32, Box<dyn std::error::Error>> {
    ffmpeg_next::init()?;
    let appdata = std::env::var_os("APPDATA").ok_or("APPDATA is missing")?;
    let mut settings =
        settings::Settings::load(PathBuf::from(appdata).join("AirPlay-Windows/settings.ini"))?;
    i18n::initialize(&mut settings);
    let data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is missing")?
        .join("AirBridge");
    let seed = settings::identity(&data.join("rust-identity.key"))?;
    let headless = std::env::args().any(|a| a == "--headless");
    let model = Arc::new(Mutex::new(media::Model {
        status: "Ready — select AirBridge in Screen Mirroring".into(),
        smooth_scaling: settings.flag("mirror_smooth_scaling", false),
        unfiltered_scaling: settings.flag("mirror_unfiltered_scaling", false),
        ..Default::default()
    }));
    let playback: Arc<dyn rust_airplay::Renderer> = if headless {
        Arc::new(rust_airplay::NullRenderer::new())
    } else {
        let mut p = media::Playback::new(model.clone());
        p.hardware = settings.flag("mirror_hwaccel", true);
        Arc::new(p)
    };
    let mut config = rust_airplay::Config::new(settings.name(), seed, playback);
    config.port = option("--port")
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(7000);
    config.advertise = !std::env::args().any(|a| a == "--no-mdns");
    if std::env::args().any(|a| a == "--loopback") {
        config.bind = "127.0.0.1".parse()?;
    }
    config.pin = rust_airplay::server::PinMode::Disabled;
    // A phone uses several concurrent control/event connections. Replacing its
    // connection at accept time would disconnect pairing and mirror setup.
    config.nohold = false;
    config.features = Some(
        (rust_airplay::server::DEFAULT_FEATURES & !((1 << 4) | (1 << 5)))
            | if settings.flag("hevc_enabled", true) {
                1 << 42
            } else {
                0
            },
    );
    let (width, height) = settings.resolution();
    config.display.width = width.into();
    config.display.height = height.into();
    config.display.max_fps = settings.number("max_fps", 60, 1, 60).into();
    config.display.refresh_rate = settings.number("refresh_rate", 60, 24, 240) as f64;
    let preview_stop = Arc::new(AtomicBool::new(false));
    let preview = if let Some(path) = option("--preview-fixture") {
        config.advertise = false;
        config.bind = "127.0.0.1".parse()?;
        config.port = 0;
        let renderer = config.renderer.clone();
        let stop = preview_stop.clone();
        Some(std::thread::spawn(move || {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                let mut input = ffmpeg_next::format::input(&path)?;
                let codec = match input
                    .streams()
                    .best(ffmpeg_next::media::Type::Video)
                    .ok_or("No video stream")?
                    .parameters()
                    .id()
                {
                    ffmpeg_next::codec::Id::H264 => rust_airplay::VideoCodec::H264,
                    ffmpeg_next::codec::Id::HEVC => rust_airplay::VideoCodec::H265,
                    _ => return Err("Preview expects Annex-B H.264/HEVC fixture".into()),
                };
                let stream = renderer.open_video(rust_airplay::VideoConfig {
                    width: None,
                    height: None,
                    codec,
                })?;
                for (_, packet) in input.packets() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    stream.push(rust_airplay::VideoFrame {
                        data: packet.data().ok_or("Empty packet")?.to_vec(),
                        timestamp: 0,
                        keyframe: packet.is_key(),
                    })?;
                    std::thread::sleep(Duration::from_millis(33));
                }
                while !stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                stream.close()?;
                Ok(())
            })();
            if let Err(e) = result {
                log::error!("fixture preview failed: {e}");
                RECEIVER_FAILED.store(true, Ordering::Release);
            }
        }))
    } else {
        None
    };
    config.on_metadata = Some(Arc::new({
        let model = model.clone();
        move |bytes| {
            let text = rust_airplay::media::dmap::text(bytes);
            media::lock(&model).metadata = text;
        }
    }));
    config.on_artwork = Some(Arc::new({
        let model = model.clone();
        move |mime, bytes| match media::artwork(mime, bytes) {
            Ok(p) => media::lock(&model).artwork = Some(Arc::new(p)),
            Err(e) => log::warn!("artwork rejected: {e}"),
        }
    }));
    let duration = option("--duration-seconds")
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    log::info!(
        "Rust receiver starting name={} port={} mdns={} root={}",
        config.name,
        config.port,
        config.advertise,
        root.display()
    );
    let work = move || -> Result<(), String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let result = runtime.block_on(rust_airplay::serve(config, async move {
            if let Some(seconds) = duration {
                tokio::select! { _ = stopped => {}, _ = tokio::time::sleep(Duration::from_secs(seconds)) => {} }
            } else { let _ = stopped.await; }
        })).map_err(|e|e.to_string());
        runtime.shutdown_timeout(Duration::from_secs(2));
        if let Err(e) = &result {
            log::error!("receiver failed: {e}");
            RECEIVER_FAILED.store(true, Ordering::Release);
        }
        result
    };
    if headless {
        work().map_err(io::Error::other)?;
        return Ok(0);
    }
    let worker = std::thread::spawn(work);
    let result = ui::receiver(model, settings);
    preview_stop.store(true, Ordering::Release);
    if let Some(preview) = preview {
        let _ = preview.join();
    }
    let _ = stop.send(());
    let service_result = worker
        .join()
        .map_err(|_| io::Error::other("Receiver thread panicked"))?;
    service_result.map_err(io::Error::other)?;
    Ok(result?)
}
fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let root = root()?;
    let is_receiver = std::env::args().any(|a| a == "--receiver");
    logging::FileLogger::install(root.join("logs").join(if is_receiver {
        "rust-receiver.log"
    } else {
        "rust-supervisor.log"
    }))?;
    std::panic::set_hook(Box::new(|p| {
        log::error!("panic: {p}");
        RECEIVER_FAILED.store(true, Ordering::Release);
    }));
    if is_receiver {
        return receiver(root);
    }
    let Some(_instance) = platform::Instance::acquire()? else {
        platform::focus();
        return Ok(0);
    };
    ui::shell(root)?;
    Ok(0)
}
fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(e) => {
            log::error!("fatal: {e}");
            if !std::env::args().any(|a| a == "--headless" || a == "--receiver") {
                platform::error(&e.to_string());
            }
            1
        }
    };
    std::process::exit(code);
}
