#!/usr/bin/env python3
"""Replay synthetic/consented audio through the actual capture finalization path."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile

from transcription_eval import digest, run_measured, scorer_fingerprint, speaker_error, word_error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True, type=Path)
    parser.add_argument('--model', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.manifest = args.manifest.resolve()
    args.model = args.model.resolve()
    manifest_hash, model_hash = digest(args.manifest), digest(args.model)
    manifest = json.loads(args.manifest.read_text())
    if manifest['provenance'] not in ['synthetic', 'consented']:
        raise ValueError('evaluation requires synthetic or explicitly consented audio')
    fixture_hashes = {}
    for case in manifest['cases']:
        for field in ['mic_audio', 'system_audio']:
            path = args.manifest.parent / case[field]
            expected = case['sha256'][field]
            if digest(path) != expected:
                raise ValueError(f'fixture checksum mismatch: {case["id"]} {field}')
            fixture_hashes[path] = expected
    # Cargo supplies native library paths when running tests; reproduce those
    # paths for direct execution so resource usage excludes the compiler.
    build = subprocess.run(['cargo', 'test', '-p', 'bruhs-capture', '--locked', '--no-run',
                            '--message-format=json'], capture_output=True, text=True, check=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
    executables = [item['executable'] for item in artifacts
                   if item.get('reason') == 'compiler-artifact'
                   and item.get('target', {}).get('name') == 'bruhs-capture'
                   and item.get('profile', {}).get('test') and item.get('executable')]
    if len(executables) != 1:
        raise ValueError('expected exactly one capture test executable')
    executable = Path(executables[0])
    binary_hash = digest(executable)
    native_paths = [path.removeprefix('native=') for item in artifacts
                    if item.get('reason') == 'build-script-executed'
                    for path in item.get('linked_paths', [])]
    with tempfile.TemporaryDirectory(prefix='bruhs-finalization-') as directory:
        raw_output = Path(directory) / 'result.json'
        env = os.environ.copy()
        env.update(BRUHS_TRANSCRIPTION_EVAL_MANIFEST=str(args.manifest),
                   BRUHS_TRANSCRIPTION_EVAL_MODEL=str(args.model),
                   BRUHS_TRANSCRIPTION_EVAL_OUTPUT=str(raw_output))
        library_env = 'DYLD_LIBRARY_PATH' if platform.system() == 'Darwin' else 'LD_LIBRARY_PATH'
        env[library_env] = os.pathsep.join(native_paths + [env.get(library_env, '')])
        # run_measured accepts argv only; a small env wrapper keeps environment
        # changes local to this child without modifying the evaluator process.
        argv = ['env'] + [f'{key}={env[key]}' for key in [
            'BRUHS_TRANSCRIPTION_EVAL_MANIFEST', 'BRUHS_TRANSCRIPTION_EVAL_MODEL',
            'BRUHS_TRANSCRIPTION_EVAL_OUTPUT', library_env]] + [str(executable),
            'postprocess::quality_eval::offline_postprocess_quality', '--ignored', '--exact']
        result, elapsed, usage = run_measured(argv)
        if result.returncode:
            raise RuntimeError(f'capture replay failed:\n{result.stdout}\n{result.stderr}')
        report = json.loads(raw_output.read_text())
    for path, expected in fixture_hashes.items():
        if digest(path) != expected:
            raise ValueError('fixture changed during evaluation')
    if digest(args.manifest) != manifest_hash or digest(args.model) != model_hash:
        raise ValueError('manifest or model changed during evaluation')
    if digest(executable) != binary_hash:
        raise ValueError('capture binary changed during evaluation')
    for field, expected in [('manifest_sha256', manifest_hash), ('model_sha256', model_hash)]:
        if report[field] != expected:
            raise ValueError(f'pipeline replay used a different {field}')
    cases = {case['id']: case for case in report['cases']}
    if cases.keys() != {case['id'] for case in manifest['cases']}:
        raise ValueError('pipeline replay returned different cases')
    for case in manifest['cases']:
        measured = cases[case['id']]
        text = ' '.join(segment['text'] for segment in measured['segments'])
        measured['word_error'] = word_error(case['reference'], text)
        measured['speaker_error'] = speaker_error(case['turns'], measured['segments'])
    report.update(recorded_at=datetime.now(timezone.utc).isoformat(),
                  binary_sha256=binary_hash, scorer_sha256=scorer_fingerprint(),
                  hardware={'platform': platform.platform(), 'machine': platform.machine()},
                  runtime={'wall_seconds': elapsed,
                           'cpu_seconds': usage.ru_utime + usage.ru_stime,
                           'peak_rss_bytes': usage.ru_maxrss * (1 if platform.system() == 'Darwin' else 1024)})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(f'Finalization evidence saved to {args.output}')


if __name__ == '__main__':
    main()
