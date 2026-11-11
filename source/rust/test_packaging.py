"""Smoke-test the deployed receiver, including a copy without the old engine."""
from pathlib import Path
import os
import shutil
import socket
import subprocess
import tempfile
import time
import ctypes
from ctypes import wintypes

ROOT = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix='airbridge-portable-') as directory:
    folder = Path(directory)
    for name in ['AirBridge.exe','avcodec-63.dll','avformat-63.dll','avutil-61.dll','swscale-10.dll','swresample-7.dll']:
        shutil.copy2(ROOT / name, folder / name)
    env = os.environ.copy()
    env.pop('AIRBRIDGE_ROOT', None)
    env['LOCALAPPDATA'] = str(folder / 'local-data')
    env['APPDATA'] = str(folder / 'settings')
    command = [str(folder / 'AirBridge.exe'),'--receiver','--headless','--loopback','--no-mdns']
    # The default DLL lookup must work without the developer SDK in PATH.
    result = subprocess.run(command + ['--port','0','--duration-seconds','1'], env=env, timeout=10)
    assert result.returncode == 0, result.returncode
    assert (folder / 'logs/rust-receiver.log').exists(), 'Logs escaped the portable folder'
    assert not (folder / 'engine').exists()
    print('PASS: portable Rust receiver runs with only its own EXE and five codec DLLs')
    kernel=ctypes.WinDLL('kernel32')
    kernel.GetUserDefaultUILanguage.restype=wintypes.WORD
    kernel.LCIDToLocaleName.argtypes=[wintypes.DWORD,wintypes.LPWSTR,ctypes.c_int,wintypes.DWORD]
    locale=ctypes.create_unicode_buffer(85)
    kernel.LCIDToLocaleName(kernel.GetUserDefaultUILanguage(),locale,85,0)
    supported=['en','ru','de','fr','es','pt','it','pl','uk','tr']
    expected=locale.value.split('-')[0].lower()
    if expected not in supported:expected='en'
    settings=folder/'settings/AirPlay-Windows/settings.ini'
    assert f'language={expected}\n' in settings.read_text(), 'First launch must persist Windows UI language or English fallback'
    settings.write_text('language=zh-CN\n',encoding='utf-8')
    result=subprocess.run(command+['--port','0','--duration-seconds','1'],env=env,timeout=10)
    assert result.returncode==0
    assert 'language=en\n' in settings.read_text(), 'Unsupported language must fall back to English'
    print('PASS: first-launch Windows language selection and unsupported-language English fallback persisted')
    with socket.socket() as occupied:
        occupied.bind(('127.0.0.1',0))
        occupied.listen()
        port = occupied.getsockname()[1]
        start = time.monotonic()
        result = subprocess.run(command + ['--port',str(port),'--duration-seconds','20'], env=env, timeout=5)
        assert result.returncode != 0
        assert time.monotonic() - start < 5
    print('PASS: occupied control port returns a failure without hanging in a modal dialog')
