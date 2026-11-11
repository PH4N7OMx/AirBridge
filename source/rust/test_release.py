"""Validate release ZIPs, the built installer and uninstall on an isolated Windows runner."""
from pathlib import Path
import hashlib
import os
import subprocess
import sys
import tempfile
import zipfile

dist = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2] / 'dist'
installer = next(dist.glob('AirBridge-*-setup.exe'))
portable = next(dist.glob('AirBridge-*-portable.zip'))
source = next(p for p in dist.glob('AirBridge-*-source.zip') if not p.name.endswith('-codec-sources.zip'))
codecs = next(dist.glob('AirBridge-*-codec-sources.zip'))
private = [str(Path(__file__).resolve().parents[2]), os.environ.get('USERPROFILE', ''), os.environ.get('COMPUTERNAME', '')]
private = [value for value in private if len(value) > 5]
installer_bytes = installer.read_bytes()
for marker in private:
    for variant in [marker, marker.replace('\\', '/')]:
        assert variant.lower().encode() not in installer_bytes.lower(), 'Local machine data in installer'
        assert variant.encode('utf-16-le') not in installer_bytes, 'Local machine data in installer'

for archive in [portable, source, codecs]:
    with zipfile.ZipFile(archive) as package:
        assert package.testzip() is None, archive
        names = package.namelist()
        relative_names = [n.partition('/')[2] for n in names]
        local_only = {
            'RELEASE_NOTES.md', 'RELEASING.md', 'RELEASE-CHECKLIST.md', 'ЧИТАТЬ.txt',
            'docs/RELEASE_NOTES.md', 'docs/RELEASING.md', 'docs/GITHUB.md',
            'source/rust/VALIDATION.md',
        }
        assert not local_only.intersection(relative_names), 'Local preparation notes were packaged'
        assert not any(n.startswith(('logs/', 'dist/', 'source/rust/tools/')) for n in relative_names)
        if archive == portable:
            assert not any(n.endswith('.zip') or '/tests/' in n for n in names), 'Runtime must not carry source/test archives'
        for name in names:
            if name.endswith('/'):
                continue
            data = package.read(name)
            for marker in private:
                for variant in [marker, marker.replace('\\', '/')]:
                    assert variant.lower().encode() not in data.lower(), f'Local machine data in {name}'
                    assert variant.encode('utf-16-le') not in data, f'Local machine data in {name}'
print('PASS archive integrity, runtime/source separation and local-machine privacy checks')

for line in (dist / 'SHA256SUMS.txt').read_text().splitlines():
    expected, name = line.split('  ', 1)
    assert hashlib.sha256((dist / name).read_bytes()).hexdigest() == expected, name

with tempfile.TemporaryDirectory(prefix='airbridge-release-') as temporary:
    root = Path(temporary)
    with zipfile.ZipFile(portable) as package:
        package.extractall(root / 'portable')
    payload = next((root / 'portable').iterdir())
    env = os.environ.copy()
    env.pop('AIRBRIDGE_ROOT', None)
    env['APPDATA'] = str(root / 'settings')
    env['LOCALAPPDATA'] = str(root / 'local')
    command = ['--receiver', '--headless', '--loopback', '--no-mdns', '--port', '0', '--duration-seconds', '1']
    subprocess.run([str(payload / 'AirBridge.exe'), *command], env=env, timeout=15, check=True)
    print('PASS actual extracted portable package startup')

    installed = root / 'installed'
    result = subprocess.run([str(installer), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', f'/DIR={installed}'], timeout=120)
    assert result.returncode == 0, result.returncode
    for name in ['AirBridge.exe', 'avcodec-63.dll', 'avformat-63.dll', 'avutil-61.dll', 'swresample-7.dll', 'swscale-10.dll']:
        assert (installed / name).read_bytes() == (payload / name).read_bytes(), name
    subprocess.run([str(installed / 'AirBridge.exe'), *command], env=env, timeout=15, check=True)
    preferences = root / 'settings/AirPlay-Windows/settings.ini'
    assert preferences.exists()
    print('PASS silent per-user installation and installed application startup')
    result = subprocess.run([str(installed / 'unins000.exe'), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'], timeout=120)
    assert result.returncode == 0, result.returncode
    assert not (installed / 'AirBridge.exe').exists(), 'Uninstaller left the application behind'
    assert preferences.exists(), 'Uninstaller removed user preferences'
    print('PASS uninstall removes application and preserves preferences')
