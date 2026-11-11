"""Run the original handshake scenarios against the Rust receiver on loopback.
The test starts and stops only its own process; no phone or multicast is needed.
"""
from pathlib import Path
import importlib.util
import os
import plistlib
import socket
import subprocess
import sys
import time
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
exe = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / 'target/debug/AirBridge.exe'
env = os.environ.copy()
env['PATH'] = str(HERE / 'vendor/ffmpeg-airbridge-9.0.2/bin') + os.pathsep + env['PATH']
temporary = tempfile.TemporaryDirectory(prefix='airbridge-protocol-')
env['AIRBRIDGE_ROOT'] = temporary.name
env['LOCALAPPDATA'] = str(Path(temporary.name) / 'local')
env['APPDATA'] = str(Path(temporary.name) / 'settings')
with socket.socket() as reservation:
    reservation.bind(('127.0.0.1', 0))
    port = reservation.getsockname()[1]
child = subprocess.Popen([str(exe), '--receiver', '--headless', '--loopback', '--no-mdns', '--port', str(port), '--duration-seconds', '90'], env=env)
try:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise RuntimeError(f'Receiver exited early: {child.returncode}')
        try:
            with socket.create_connection(('127.0.0.1', port), timeout=.25):
                break
        except OSError:
            time.sleep(.1)
    else:
        raise RuntimeError('Receiver did not listen')
    spec = importlib.util.spec_from_file_location('upstream_tests', HERE / 'tests/support/test_pair_verify.py')
    suite = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(suite)
    suite.PORT = port
    try:
        suite.main()
    except SystemExit as result:
        if result.code not in (None,0):
            raise
    # iOS sends audioMode while the mirror session is alive. A mode notification
    # must succeed and leave this same TCP connection usable for feedback.
    with socket.create_connection(('127.0.0.1',port),timeout=2) as connection:
        reader = connection.makefile('rb')
        for sequence,(target,body) in enumerate([
            ('/audioMode',b''),
            ('/audioMode',plistlib.dumps({'audioMode':'default'},fmt=plistlib.FMT_BINARY)),
            ('/feedback',b'')],1):
            connection.sendall(f'POST {target} RTSP/1.0\r\nCSeq: {sequence}\r\nContent-Type: application/x-apple-binary-plist\r\nContent-Length: {len(body)}\r\n\r\n'.encode()+body)
            assert b' 200 ' in reader.readline(), f'{target} rejected'
            headers = {}
            while (line:=reader.readline()) != b'\r\n':
                assert line, 'Server closed after audioMode'
                key,value = line.decode().split(':',1)
                headers[key.lower()] = value.strip()
            assert headers.get('connection','').lower() != 'close'
            reader.read(int(headers.get('content-length','0')))
    print('PASS: audioMode (empty and plist) returns 200; same connection continues with feedback')
    with socket.create_connection(('127.0.0.1', port), timeout=2) as connection:
        reader = connection.makefile('rb')
        for sequence, (method, target, body) in enumerate([
            ('TEARDOWN', '/stream', plistlib.dumps({'streams': [{'type': 96}]}, fmt=plistlib.FMT_BINARY)),
            ('POST', '/feedback', b''),
            ('TEARDOWN', '/stream', plistlib.dumps({'streams': [{'type': 110}]}, fmt=plistlib.FMT_BINARY)),
            ('POST', '/feedback', b''),
            ('TEARDOWN', '/stream', b'')], 1):
            connection.sendall(f'{method} {target} RTSP/1.0\r\nCSeq: {sequence}\r\nContent-Length: {len(body)}\r\n\r\n'.encode() + body)
            assert b' 200 ' in reader.readline(), f'{method} {target} rejected'
            headers = {}
            while (line := reader.readline()) != b'\r\n':
                assert line, 'Server closed after stream-only TEARDOWN'
                key, value = line.decode().split(':', 1)
                headers[key.lower()] = value.strip()
            reader.read(int(headers.get('content-length', '0')))
            assert (headers.get('connection', '').lower() == 'close') == (sequence == 5)
        assert reader.read(1) == b'', 'Full TEARDOWN did not close connection'
    print('PASS: audio/video stream-only TEARDOWN preserves feedback; full TEARDOWN closes connection')
finally:
    child.terminate()
    child.wait(timeout=5)
    temporary.cleanup()
