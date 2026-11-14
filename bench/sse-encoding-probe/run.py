"""Build separate constructor/allocation probes, then run without builders."""
import fcntl
import hashlib
import json
import math
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATE = Path(__file__).resolve().parent
os.chdir(ROOT)
if len(sys.argv) != 2:
    raise SystemExit('Usage: python3 bench/sse-encoding-probe/run.py target/<new-study-directory>')
out = Path(sys.argv[1]).resolve()
assert out.is_relative_to(ROOT / 'target') and not out.exists(), 'Use a new directory under target; retain previous studies'
out.mkdir(parents=True)
sha = lambda path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
paths = sorted([p for p in CRATE.rglob('*') if p.is_file() and 'target' not in p.relative_to(CRATE).parts])
paths += [ROOT / 'Cargo.lock', ROOT / 'bruhs-core/src/models/task.rs']
paths += sorted(p for p in (ROOT / 'third_party/axum').rglob('*') if p.is_file())
manifest = {str(path.relative_to(ROOT)): sha(path) for path in paths}
(out / 'sources.json').write_text(json.dumps({'captured_unix_ns': time.time_ns(), 'source_sha256': manifest,
    'scope': 'Probe, model, lockfile and vendored Axum snapshot; not an exhaustive transitive build graph.'}, indent=2) + '\n')
for path in paths:
    copy = out / 'source' / path.relative_to(ROOT)
    copy.parent.mkdir(parents=True, exist_ok=True)
    copy.write_bytes(path.read_bytes())

def verify():
    assert all(sha(ROOT / path) == expected for path, expected in manifest.items()), 'Source changed during study'

def command(name, args, binary=None):
    verify()
    start = time.time_ns()
    with (out / (name + '.log')).open('w') as log:
        result = subprocess.run(args, stdout=log, stderr=subprocess.STDOUT)
    receipt = {'args': args, 'exit_code': result.returncode, 'started_unix_ns': start,
               'finished_unix_ns': time.time_ns(), 'log_sha256': sha(out / (name + '.log')),
               'sources_sha256': sha(out / 'sources.json')}
    if result.returncode == 0 and binary:
        (out / binary).write_bytes((ROOT / 'target/release/sse-encoding-probe').read_bytes())
        (out / binary).chmod(0o755)
        receipt['binary_sha256'] = sha(out / binary)
    (out / (name + '-receipt.json')).write_text(json.dumps(receipt, indent=2) + '\n')
    assert result.returncode == 0, f'{name} failed; inspect retained log'
    verify()
    return receipt

base = ['--locked', '--manifest-path', str(CRATE / 'Cargo.toml'), '--target-dir', str(ROOT / 'target')]
command('test', ['cargo', 'test', *base, '--all-features'])
command('clippy', ['cargo', 'clippy', *base, '--all-targets', '--all-features', '--', '-D', 'warnings'])
command('timing-build', ['cargo', 'build', *base, '--release'], 'timing')
command('allocation-build', ['cargo', 'build', *base, '--release', '--features', 'count-allocations'], 'allocations')

def workers():
    found = []
    for line in subprocess.check_output(['ps', '-axo', 'pid=,comm='], text=True).splitlines():
        parts = line.split(None, 1)
        if len(parts) != 2: continue
        path = Path(parts[1])
        if path.name in ['cargo', 'rustc', 'rustc-real'] or (str(ROOT / 'target') in str(path) and (path.name.startswith('performance-') or path.name in ['timing', 'allocations', 'sse-encoding-probe'])):
            found.append(line.strip())
    return found

assert not workers(), workers()
locks = []
for profile in ['debug', 'release', 'wasm32-unknown-unknown/debug', 'wasm32-unknown-unknown/release']:
    lock = (ROOT / 'target' / profile / '.cargo-lock').open('a+')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    locks.append(lock)
cohorts = {}
for mode, order in [('timing', 'forward'), ('allocations', 'forward'), ('allocations', 'reverse'), ('timing', 'reverse')]:
    assert not workers(), workers()
    tag = mode + '-' + order
    args = [str(out / mode)] + (['reverse'] if order == 'reverse' else [])
    receipt = command(tag, args)
    assert not workers(), workers()
    text = (out / (tag + '.log')).read_text()
    prefix = 'SSE_ENCODING ' if mode == 'timing' else 'SSE_ALLOC '
    rows = [json.loads(line.removeprefix(prefix)) for line in text.splitlines() if line.startswith(prefix)]
    assert len(rows) == 306 and 'SSE_CONTRACTS ' in text
    receipt.update({'binary_sha256': sha(out / mode), 'lock_paths': [str(lock.name) for lock in locks], 'workers_postflight': workers(), 'rows': rows})
    (out / (tag + '-receipt.json')).write_text(json.dumps(receipt, indent=2) + '\n')
    cohorts[tag] = rows
    print(tag, 'passed', flush=True)
for lock in locks:
    fcntl.flock(lock, fcntl.LOCK_UN)
    lock.close()
comparisons = {}
fingerprints = {}
for order in ['forward', 'reverse']:
    rows = {}
    for row in cohorts['timing-' + order]:
        key = f"{row['shape']}/{row['records']}/{row['named']}"
        samples = sorted(row['samples_ms'])
        assert len(samples) == 30 and row['warmups'] == 5
        rows.setdefault(key, {})[row['method']] = {**row, 'summary_ms': {f'p{q}': samples[math.ceil(len(samples) * q / 100) - 1] for q in [50, 95, 99]}}
    for row in cohorts['allocations-' + order]:
        key = f"{row['shape']}/{row['records']}/{row['named']}"
        rows[key][row['method']]['counts'] = row['counts']
        assert row['counts']['live_bytes_at_end'] == 0
    for key, methods in rows.items():
        assert len(methods) == 9
        for row in methods.values():
            signature = (row['raw_frame_sha256'], row['raw_frame_bytes'])
            assert key not in fingerprints or fingerprints[key] == signature
            fingerprints[key] = signature
    comparisons[order] = rows
for key, methods in comparisons['forward'].items():
    for method, row in methods.items():
        assert row['counts'] == comparisons['reverse'][key][method]['counts']
(out / 'comparison.json').write_text(json.dumps({'status': 'passed', 'comparisons': comparisons,
    'primary_intervals': 18360, 'excluded_warmups': 3060, 'allocation_observations': 612,
    'scope': 'Separate constructor-only timing/allocation binaries. Actual task models; full raw-frame/error parity. No registry/network/client cost, RSS, native/browser frames or whole-action acceptance.'}, indent=2) + '\n')
print('Verified both method/data orders, raw frames, allocation counts and all constructor samples.', flush=True)
