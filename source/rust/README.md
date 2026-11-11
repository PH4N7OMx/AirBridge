# AirBridge Rust

The deployed `../../AirBridge.exe` contains both the Rust tray supervisor and the Rust
AirPlay receiver. The supervisor starts the same executable with `--receiver`. No legacy launcher or
separate receiver engine is required.

AirPlay discovery, pairing, FairPlay setup, stream decryption and media transport use a
locally patched GPL-3.0-or-later copy of rust-airplay 0.3.2 in `crates/rust-airplay`.
Playback, settings, the Windows interface and supervision are Rust code in
`src`. FFmpeg remains a native codec/muxer dependency, and Windows provides the native
window and sound APIs. This is not a claim that every third-party dependency is Rust.

## Run

Run `../../AirBridge.exe` on Windows 10 1809+ or Windows 11 x64. Keep the five codec DLLs
beside that executable. Neither the old `engine` folder nor .NET is required by this build.

Connect the phone and PC to the same local network, then select **AirBridge** under
Control Center → Screen Mirroring. The receiver prefers TCP 7000, automatically chooses
another available port if another app owns it, and advertises
`_airplay._tcp` and `_raop._tcp`. Bonjour is not required.
The same discovery records are refreshed every 30 seconds without interrupting playback.
Logs include actual mDNS announcements, interface changes, errors and name conflicts.

Use **Настройки** in the toolbar for name, resolution, FPS, GPU decoding and HEVC. Changes save
atomically and restart the receiver. **Auto resolution is on by default**: it advertises
a 4096 × 4096 maximum for either orientation, and the phone chooses its actual stream.
This does not force the phone's full native resolution. Disable Auto to use the manual
width/height limits; new manual defaults are 1920 × 1080. Explicit manual limits are
preserved; the legacy automatic 2622 × 2622 default is replaced in the form.
The frame cap is 60 FPS, including older settings files containing max_fps=120.
Numeric fields accept digits only, with 1–60 FPS and 320–8192 pixels per dimension.
**F11** toggles fullscreen, **Esc** exits
fullscreen, and **R** rotates the displayed image. Recording and its Ctrl+R shortcut
have been removed from the app; no recorder is created during normal operation.

The interface uses iOS-inspired light/dark palettes, grouped settings, keyboard-focusable
switches and privately embedded Inter 4.1 fonts (SIL OFL 1.1). No font installation is
required. Symbols are original vector geometry; no Apple fonts or SF Symbols are bundled.
The **Вид** toolbar menu contains rotation, fullscreen and the three scaling modes.
Fullscreen hides the toolbar, leaving the full client area for video. Status is in the
header. Settings use a draggable custom header with a close button; Escape also closes.
Switch thumbs animate for 180 ms; their track color changes immediately.
Theme defaults to Windows app personalization and follows WM_SETTINGCHANGE updates.
The System / Light / Dark choice is saved immediately without restarting the receiver.
Theme changes use a 180 ms composited crossfade over the complete application windows,
covering the synchronous repaint of native child controls. No per-control recoloring
is exposed during the transition; the temporary overlay is mouse-transparent.

**Language** is chosen from the Windows display language on first launch; an unsupported
language falls back to English. The saved language overrides subsequent Windows changes.
Settings offer English, Russian, German, French, Spanish, Portuguese, Italian, Polish,
Ukrainian and Turkish; changes apply immediately to the receiver, menus and tray.
The UTF-8 catalog is embedded from `locales/ui.tsv`, with complete-catalog tests.
The More menu contains reconnect/network/log/quit actions; Settings has its own button.

The application icon and version metadata are compiled by `build.rs` using Windows SDK
`rc.exe` (or `RC` override). The ICO is embedded; external image files are not needed
for the portable runtime. Source artwork and license notes are in `../../assets/icons`.
Settings remember their screen position; the receiver remembers position, normal size
and maximized state. Placement is restored within an available monitor's work area.
Fullscreen does not overwrite the saved normal placement. Layout state is separate
from receiver preferences in `%LOCALAPPDATA%/AirBridge/window-state.ini`.

The tray menu provides focus, reconnect, logs and exit. Closing the receiver normally
also exits the supervisor. A crash or three failed `/info` checks triggers recovery;
after two automatic recoveries per minute, use the tray's manual reconnect action.
The UI remains responsive while the worker checks or stops the receiver. The child is
assigned to a kill-on-close Windows job atomically during creation, preventing orphans.

**••• → Доступ в локальной сети…** requests Windows administrator approval to add an
application firewall rule for **local-subnet traffic on Private/Domain profiles**. No
firewall changes are made merely by starting or testing the app. Existing block rules or
administrator policies may still prevent access. This action was not executed in QA.

## Local data

- Existing preferences: `%APPDATA%/AirPlay-Windows/settings.ini`; unknown keys are retained.
- New receiver identity: `%LOCALAPPDATA%/AirBridge/rust-identity.key`. Invalid identity files
  are reported instead of silently replaced. Existing identity data is not replaced during release preparation.
- Rotating diagnostics: `../../logs/rust-supervisor.log` and `rust-receiver.log`.
- Legacy C#/C++ applications and backups have been removed from this repository.
- The retained handshake test and its upstream attribution are in `tests/support/`.

## Compatibility and remaining limits

This is a new Rust implementation with the core receiver functionality; it does not
duplicate every advanced option of the previous C++ UI.

- H.264 and HEVC use D3D11VA when available, with software decoding when device creation
  fails. GPU frames are transferred for double-buffered GDI display. The previous
  zero-copy D3D11 presentation path and its VSync/latency controls are not ported.
  Compressed frames now use a bounded, ordered queue with asynchronous TCP backpressure.
  A separate presentation worker keeps just the latest decoded image, so slow GPU
  readback/color conversion cannot discard HEVC reference pictures and freeze the stream.
  BGRA conversion follows the window size, reducing display work for high-resolution
  streams while keeping source resolution for recording. Filtered reduction suppresses
  aliasing in fine detail; final GDI stretching uses halftone filtering.
  **Display → Sharp scaling (text)** is now the default Lanczos filter, retaining
  more edge contrast than area averaging. **Smooth scaling** restores the softer
  area filter. Selection applies to the next picture without reconnecting and is saved.
- Audio supports the FairPlay/AES stream path with AAC-ELD and ALAC through FFmpeg and
  Windows output. Resampling allocates for the output rate and internal delay. A 40 ms
  jitter buffer smooths scheduling, with a 120 ms cap before returning to 40 ms.
  Device hot-unplug may require reconnecting the receiver.
- Legacy ANNOUNCE/UDP control framing is implemented. RSA-wrapped AirPlay 1 audio-key
  decryption is not implemented; a successful legacy setup test is not an audio proof.
- The old recording implementation remains in source for media regression tests only.
- Metadata and bounded PNG/JPEG artwork decoding are supported. Manual rotation affects
  display, not recorded video. Rotation/display scaling are not a device-orientation sensor.
- Direct HLS/video playback and DRM are not advertised or implemented. The app cannot
  display video that the sender does not transmit.
- Real iPhone/iPad/Mac playback, live A/V synchronization, sustained high-resolution
  performance and TikTok route switching still require testing with those devices.

## Build

Install Rust for `x86_64-pc-windows-msvc`, Visual Studio C++ build tools/Windows SDK,
PowerShell 7, Git for Windows, Python 3 and Windows `tar`. On a fresh checkout:

```powershell
pwsh -ExecutionPolicy Bypass -File source/rust/Setup-Dependencies.ps1
pwsh -ExecutionPolicy Bypass -File source/build.ps1
```

`Setup-Dependencies.ps1` builds a project-specific FFmpeg 9.0.2 SDK from pinned
FFmpeg, x264 and zlib source archives. URLs, exact revisions and SHA-256 values are
recorded in `codec-sources.json`; build recipes are `Build-FFmpeg.ps1` and
`build-codecs.sh`. No Gyan full-build DLLs are used in new releases. Microsoft C++
tools and Git Bash are required; the helper downloads checksum-pinned make, NASM
and pkgconf build tools into the ignored `tools` directory. Subsequent setup runs
verify the cached SDK against its recipe fingerprint and DLL checksums.

libclang 18.1.1 is a build tool installed locally under `tools`; it is not shipped.
`build.ps1 -Deploy` copies the Rust executable and five codec DLLs to the package root.
Version tags trigger `../../.github/workflows/release.yml`, which builds packages
and creates an unpublished release draft. Local packaging uses
`../../scripts/Prepare-Release.ps1` (PowerShell 7; `-Installer` also needs Inno Setup).
The build remaps profile/workspace paths to neutral locations. Packaging rejects
files containing the current build machine's profile, workspace or hostname.

For direct Cargo commands, from this directory:

```powershell
$env:FFMPEG_DIR = Join-Path $PWD 'vendor/ffmpeg-airbridge-9.0.2'
$env:LIBCLANG_PATH = Join-Path $PWD 'tools/clang/clang/native'
$env:PATH = "$env:FFMPEG_DIR/bin;$env:PATH"
cargo test --workspace --lib --bins --tests --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked
```

## Validation

Run the following automated checks from the package root:

```powershell
py -3 source/rust/test_protocol.py AirBridge.exe
py -3 source/rust/test_packaging.py
py -3 source/rust/test_ui.py
pwsh -ExecutionPolicy Bypass -File source/rust/Test-Stability.ps1
```

`test_protocol.py` needs Python `cryptography`, and `test_ui.py` needs Pillow. The latter
opens the receiver using a generated test fixture and captures only its windows with
PrintWindow/BitBlt; QA images are saved only under the ignored `tools/ui-qa`.
Its runtime logs and preferences are isolated in a temporary profile. The lifecycle script intentionally kills and suspends its own test receiver;
close AirBridge and stop any phone stream before running it.

## Dependency provenance

- rust-airplay 0.3.2: https://github.com/rust-airplay/rust-airplay — GPL-3.0-or-later;
  source, license and upstream notices included in `crates/rust-airplay`.
- ffmpeg-next 9.0.0: https://github.com/zmwangx/rust-ffmpeg — WTFPL wrapper.
- FFmpeg 9.0.2: source-built shared GPLv3 configuration; exact FFmpeg/x264/zlib
  inputs and checksums are in `codec-sources.json`. Matching source archives and
  recipes accompany release assets.
- CPAL 0.17.3: https://github.com/RustAudio/cpal — Apache-2.0.
- Tokio: https://github.com/tokio-rs/tokio — MIT.
- windows-sys: https://github.com/microsoft/windows-rs — MIT OR Apache-2.0.

Full resolved crate versions and registry checksums are in `Cargo.lock`.
See `../../THIRD_PARTY_NOTICES_RUST.md` and the retained `tests/support/UPSTREAM_NOTICES.md`.
