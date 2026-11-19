import json
import os
from pathlib import Path
import subprocess
import time
from urllib.request import urlopen

root = Path('Bruhs' if os.name == 'nt' else '/opt/bruhs').resolve()
commit = '9850730b871b730395557642f9137eb1f57dc7c0'
ext = '.exe' if os.name == 'nt' else ''
app = subprocess.Popen([root / ('bruhs-desktop' + ext)], cwd=root)
try:
    for attempt in range(60):
        assert app.poll() is None, f'desktop exit {app.returncode}'
        try:
            for port, path, name in [(3001, '/api/health', 'bruhs-server'), (3002, '/health', 'bruhs-capture')]:
                with urlopen(f'http://127.0.0.1:{port}{path}', timeout=2) as response:
                    health = json.load(response)
                    assert health['app'] == name and health['build'] == commit, health
            with urlopen('http://127.0.0.1:3001/', timeout=2) as index:
                assert b'data-bruhs-boot' in index.read()
            break
        except OSError:
            time.sleep(1)
    else:
        raise RuntimeError('Bundled services unavailable')
    time.sleep(5)
    assert app.poll() is None
    print(f'Desktop and both helpers started from {commit}, web root served')
finally:
    app.terminate()
