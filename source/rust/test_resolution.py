"""Check the actual advertised resolution in default Auto and saved manual modes."""
from pathlib import Path
import os
import plistlib
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix='airbridge-resolution-') as directory:
    env = os.environ.copy()
    env['APPDATA'] = str(Path(directory)/'settings')
    env['LOCALAPPDATA'] = str(Path(directory)/'local')
    settings = Path(env['APPDATA'])/'AirPlay-Windows/settings.ini'
    settings.parent.mkdir(parents=True)
    for mode,expected in [(None,(4096,4096)),('0',(1440,3200)),('1',(4096,4096))]:
        settings.write_text('max_fps=120\nmirror_width=1440\nmirror_height=3200\n'+('' if mode is None else f'mirror_auto_resolution={mode}\n'),encoding='utf-8')
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1',0))
            port = reservation.getsockname()[1]
        child = subprocess.Popen([str(ROOT/'AirBridge.exe'),'--receiver','--headless','--loopback','--no-mdns','--port',str(port),'--duration-seconds','10'],env=env)
        try:
            deadline = time.monotonic()+5
            while True:
                try:
                    connection = socket.create_connection(('127.0.0.1',port),timeout=1)
                    break
                except OSError:
                    if child.poll() is not None or time.monotonic()>=deadline:raise
                    time.sleep(.05)
            with connection:
                connection.sendall(b'GET /info RTSP/1.0\r\nCSeq: 1\r\n\r\n')
                reader = connection.makefile('rb')
                assert b'200' in reader.readline()
                headers = {}
                while (line:=reader.readline()) != b'\r\n':
                    if not line:raise RuntimeError('Truncated response')
                    key,value = line.decode().split(':',1)
                    headers[key.lower()] = value.strip()
                info = plistlib.loads(reader.read(int(headers['content-length'])))
                display = info['displays'][0]
                assert display['maxFPS'] == 60, display
                assert (display['width'],display['height']) == expected
                assert (display['widthPixels'],display['heightPixels']) == expected
            print(f'PASS: mode={mode or "default Auto"} advertises {expected[0]} x {expected[1]}')
        finally:
            if child.poll() is None:child.terminate()
            child.wait(timeout=5)
