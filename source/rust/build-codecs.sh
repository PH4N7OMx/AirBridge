#!/usr/bin/env bash
set -euo pipefail
work=$(cygpath -u "$1")
destination=$(cygpath -u "$2")
jobs=${3:-8}
cd "$work"
mkdir -p sdk/include sdk/lib/pkgconfig
cd zlib
nmake.exe -nologo -A -f win32/Makefile.msc zlib.lib 'CFLAGS=-nologo -MT -O2'
cp zlib.h zconf.h ../sdk/include/
# FFmpeg's MSVC build can define Z_HAVE_UNISTD_H=0; zlib tests defined(),
# so remove the unavailable POSIX include from the installed header.
sed -i '/include <unistd.h>/d' ../sdk/include/zconf.h
cp zlib.lib ../sdk/lib/
cat > ../sdk/lib/pkgconfig/zlib.pc <<'PC'
prefix=../sdk
libdir=${prefix}/lib
includedir=${prefix}/include
Name: zlib
Description: zlib compression library
Version: 1.4
Libs: -L${libdir} -lzlib
Cflags: -I${includedir}
PC
cd ../x264
if [ -f config.mak ]; then make clean; fi
CC=cl ./configure --host=x86_64-w64-mingw32 --prefix=../sdk \
  --enable-static --disable-cli --disable-opencl --disable-avs \
  --disable-lavf --disable-swscale --disable-ffms --disable-gpac --disable-lsmash \
  --bit-depth=8 --chroma-format=420 --extra-cflags=-MT
make -j"$jobs"
make install
cp ../sdk/lib/libx264.lib ../sdk/lib/x264.lib
export PKG_CONFIG_PATH=../sdk/lib/pkgconfig
pkgconf --exists x264 zlib
cd ../ffmpeg
if [ -f ffbuild/config.mak ]; then make clean; fi
./configure --toolchain=msvc --arch=x86_64 --prefix=../sdk \
  --disable-everything --disable-autodetect --disable-programs --disable-doc \
  --disable-debug --disable-static --enable-shared --disable-avdevice --disable-avfilter \
  --enable-gpl --enable-version3 --enable-libx264 --enable-zlib --pkg-config=pkgconf \
  --extra-cflags=-I../sdk/include --extra-ldflags=-LIBPATH:../sdk/lib \
  --enable-decoder=h264,hevc,aac,aac_latm,alac,png,mjpeg \
  --enable-encoder=libx264,aac \
  --enable-parser=h264,hevc,aac,aac_latm,png,mjpeg \
  --enable-demuxer=h264,hevc,mov,aac,image2,png_pipe,mjpeg \
  --enable-muxer=mp4,mov,adts,matroska --enable-protocol=file \
  --enable-d3d11va --enable-hwaccel=h264_d3d11va,h264_d3d11va2,hevc_d3d11va,hevc_d3d11va2
make -j"$jobs"
make install
mkdir -p "$destination"
cp -R ../sdk/include ../sdk/lib ../sdk/bin "$destination/"
