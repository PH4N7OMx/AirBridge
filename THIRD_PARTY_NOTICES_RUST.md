# AirBridge Rust dependency notices

AirBridge's Rust application code is GPL-3.0-or-later; the GPL text is in `LICENSE`.

The protocol receiver is based on **rust-airplay 0.3.2**, licensed GPL-3.0-or-later.
Its source and `THIRD_PARTY_NOTICES.md` are included in
`source/rust/crates/rust-airplay`. Preserve those notices, including the provenance of
the UxPlay/PlayFair translations, when redistributing this build.

The five codec DLLs beside AirBridge.exe are a project-specific **FFmpeg 9.0.2**
shared build, configured under **GPL-3.0-or-later** (`--enable-gpl --enable-version3`).
They are built from source by `source/rust/Build-FFmpeg.ps1` and `build-codecs.sh`.
The only statically incorporated external codec libraries are:

- **x264**, GPL-2.0-or-later; source revision `c24e06c2e184345ceb33eb20a15d1024d9fd3497`.
- **zlib 1.4**, zlib license; its copyright and license are retained in the source archive
  and runtime `licenses/ZLIB-README.txt`.

FFmpeg source revision: https://github.com/FFmpeg/FFmpeg/commit/946fcce07b6dcd0331c8cc609192aeff5e1924f8
Exact download URLs and SHA-256 checksums for all three source archives and the build
helper tools are in `source/rust/codec-sources.json`. FFmpeg's GPL text and build
configuration summary are in `source/rust/FFMPEG-LICENSE.txt` and `FFMPEG-BUILD.txt`.
The recipe removes the unavailable `unistd.h` include from the installed zlib header
for MSVC compatibility; pristine source archives are preserved unchanged.

Every binary release is accompanied by `AirBridge-VERSION-codec-sources.zip`, containing
those exact FFmpeg/x264/zlib archives, build recipes, configuration and shipped DLL hashes.
`AirBridge-VERSION-source.zip` contains the application, patched rust-airplay, assets and
vendored Cargo dependencies with their original notices; the included Cargo configuration
uses those vendored sources. Download the matching source assets from the **same GitHub
release** as the binary. Microsoft compiler/Windows SDK, Git Bash and checksum-pinned
make/NASM/pkgconf are build prerequisites, not additional bundled runtime codec libraries.
No Gyan full-build DLLs are included in the new source-built runtime.

Other Rust dependencies include ffmpeg-next (WTFPL), CPAL (Apache-2.0), Tokio (MIT),
windows-sys (MIT OR Apache-2.0), log (MIT OR Apache-2.0), getrandom (MIT OR Apache-2.0)
and their dependencies. Exact versions and source registry checksums are in
`source/rust/Cargo.lock`. The local compiler support package libclang is a build tool
and is not required or loaded by the deployed application.

The retained handshake test derives from AirPlay-Windows checkout `70203fd`, GPL-3.0.
Its provenance and upstream notices are in `source/rust/tests/support/`. The old C++
engine and C# launcher are no longer distributed.

The interface embeds unmodified **Inter 4.1 Regular and SemiBold** by Rasmus Andersson,
under the SIL Open Font License 1.1. Font files and the required copyright/license
notice are in `assets/fonts/`. Source: https://github.com/rsms/inter/releases/tag/v4.1
The font is registered privately in the receiver process; no system installation is made.
Interface symbols are original line geometry drawn in `src/ui_style.rs`; no SF Pro,
SF Symbols, Apple logos or other Apple design resources are bundled.

The AirBridge application icon in `assets/icons/` was created for this project with
the built-in imagegen tool. The project license applies to that artwork; it is not
Apple artwork. Its master PNG and Windows ICO are included with the source assets.
