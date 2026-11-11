"""Verify real multicast announcements recur without replacing the control server."""
import os
from pathlib import Path
import re
import socket
import struct
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
# Ask the routing table for the LAN interface, without transmitting a packet.
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as route:
    route.connect(('192.0.2.1', 9))
    interface = route.getsockname()[0]
listener = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(('', 5353))
listener.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
                   socket.inet_aton('224.0.0.251') + socket.inet_aton(interface))
listener.settimeout(.5)
with tempfile.TemporaryDirectory(prefix='airbridge-discovery-') as directory:
    folder = Path(directory)
    env = os.environ.copy()
    env['APPDATA'] = str(folder/'settings')
    env['LOCALAPPDATA'] = str(folder/'local')
    env['AIRBRIDGE_ROOT'] = str(folder)
    settings = Path(env['APPDATA'])/'AirPlay-Windows/settings.ini'
    settings.parent.mkdir(parents=True)
    name = f'Discovery-QA-{os.getpid()}'
    settings.write_text(f'receiver_name={name}\nname={name}\n',encoding='utf-8')
    child = subprocess.Popen([str(ROOT/'AirBridge.exe'),'--receiver','--headless',
                              '--port','0','--duration-seconds','38'],env=env)
    observed = []
    start = time.monotonic()
    try:
        while time.monotonic()-start < 35:
            assert child.poll() is None, 'Receiver exited during discovery refresh'
            try:
                data, peer = listener.recvfrom(9000)
            except socket.timeout:
                continue
            if peer[0] == interface and name.encode() in data and struct.unpack_from('!H',data,2)[0] & 0x8000:
                observed.append(time.monotonic()-start)
        assert observed and max(observed)-min(observed) >= 25, f'Missing periodic multicast announcements: {observed}'
        logs = (folder/'logs/rust-receiver.log').read_text()
        assert 'discovery announced' in logs, 'No actual announcement events logged'
        port = int(re.search(r'control\s+0\.0\.0\.0:(\d+)',logs).group(1))
        with socket.create_connection(('127.0.0.1',port),timeout=2) as connection:
            connection.sendall(b'GET /info RTSP/1.0\r\nCSeq: 1\r\n\r\n')
            assert b' 200 ' in connection.recv(512), 'Control server changed or stopped'
        print(f'PASS: repeated multicast announcements over {max(observed)-min(observed):.1f}s; control port {port} stays responsive')
        child.wait(timeout=8)
        assert child.returncode == 0
        print('PASS: discovery maintenance stops on normal receiver shutdown')
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=5)
        listener.close()
