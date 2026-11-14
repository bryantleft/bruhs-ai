#!/usr/bin/env python3
"""Measure production ASR/channel attribution on an explicit local evaluation corpus."""
import argparse
from datetime import datetime, timezone
import hashlib
import inspect
import json
import math
import os
import platform
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
import unicodedata


def words(text):
    text = unicodedata.normalize('NFC', text)
    return re.findall(r"[^\W_]+(?:'[^\W_]+)*", text.replace('’', "'").casefold(), flags=re.UNICODE)


def word_error(reference, hypothesis):
    expected, observed = words(reference), words(hypothesis)
    previous = list(range(len(observed) + 1))
    for i, token in enumerate(expected, 1):
        current = [i]
        for j, other in enumerate(observed, 1):
            current.append(min(current[-1] + 1, previous[j] + 1,
                               previous[j - 1] + (token != other)))
        previous = current
    errors = previous[-1]
    return {'errors': errors, 'reference_words': len(expected), 'hypothesis_words': len(observed),
            'word_error_rate': errors / len(expected) if expected else None,
            'false_silence_words': len(observed) if not expected else 0}


def speaker_error(reference, hypothesis):
    """Known-channel duration error, including misses, false alarms and overlap."""
    intervals = reference + hypothesis
    for segment in intervals:
        if not (math.isfinite(segment['start_ms']) and math.isfinite(segment['end_ms'])
                and 0 <= segment['start_ms'] < segment['end_ms']):
            raise ValueError('speaker intervals must have finite, ordered nonnegative times')
    edges = sorted({s[k] for s in intervals for k in ['start_ms', 'end_ms']})
    reference_ms = missed_ms = false_alarm_ms = confusion_ms = 0.0
    for start, end in zip(edges, edges[1:]):
        truth = {s['speaker_id'] for s in reference if s['start_ms'] <= start < s['end_ms']}
        predicted = {s['speaker_id'] for s in hypothesis if s['start_ms'] <= start < s['end_ms']}
        span = end - start
        reference_ms += len(truth) * span
        missed_ms += max(0, len(truth) - len(predicted)) * span
        false_alarm_ms += max(0, len(predicted) - len(truth)) * span
        confusion_ms += (min(len(truth), len(predicted)) - len(truth & predicted)) * span
    error_ms = missed_ms + false_alarm_ms + confusion_ms
    return {'reference_speaker_ms': reference_ms, 'missed_ms': missed_ms,
            'false_alarm_ms': false_alarm_ms, 'confusion_ms': confusion_ms,
            'error_ms': error_ms, 'error_rate': error_ms / reference_ms if reference_ms else None}


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def scorer_fingerprint():
    source = '\n'.join(inspect.getsource(function) for function in [words, word_error, speaker_error])
    return hashlib.sha256(source.encode()).hexdigest()


def run_measured(argv):
    if not hasattr(os, 'wait4'):
        raise RuntimeError('per-process resource measurement requires a platform with wait4')
    started = time.monotonic()
    with tempfile.TemporaryFile(mode='w+t') as stdout, tempfile.TemporaryFile(mode='w+t') as stderr:
        process = subprocess.Popen(argv, stdout=stdout, stderr=stderr)
        try:
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
        except BaseException:
            process.kill()
            process.wait()
            raise
        elapsed = time.monotonic() - started
        stdout.seek(0)
        stderr.seek(0)
        result = subprocess.CompletedProcess(argv, process.returncode, stdout.read(), stderr.read())
    return result, elapsed, usage


def compare(baseline, candidate):
    for field in ['manifest_sha256', 'model_sha256', 'model_language', 'scorer_sha256']:
        if baseline.get(field) is None or baseline.get(field) != candidate.get(field):
            raise ValueError(f'cannot compare different {field}')
    old = {case['id']: case for case in baseline['cases']}
    new = {case['id']: case for case in candidate['cases']}
    if old.keys() != new.keys():
        raise ValueError('cannot compare different evaluation cases')
    return {id: {
        'speech_word_errors_delta': new[id]['words']['errors'] - old[id]['words']['errors']
            if new[id]['words']['reference_words'] else None,
        'false_silence_words_delta': new[id]['words']['false_silence_words'] - old[id]['words']['false_silence_words'],
        'speaker_error_ms_delta': new[id]['speakers']['error_ms'] - old[id]['speakers']['error_ms'],
        'inference_ms_delta': new[id]['inference_ms'] - old[id]['inference_ms'],
    } for id in old}


def evaluate(binary, model, manifest_path, language, variant='candidate'):
    manifest_bytes = manifest_path.read_bytes()
    identities = {'manifest_sha256': hashlib.sha256(manifest_bytes).hexdigest(),
                  'model_sha256': digest(model), 'runner_sha256': digest(binary)}
    manifest = json.loads(manifest_bytes)
    if manifest.get('provenance') not in ['synthetic', 'consented'] or not manifest.get('provenance_note'):
        raise ValueError('corpus must document synthetic generation or consented provenance')
    cases = manifest['cases']
    if not cases or any(not case['id'] for case in cases) or len({case['id'] for case in cases}) != len(cases):
        raise ValueError('evaluation case identities must be nonempty and unique')
    fixture_identities = {}
    for case in cases:
        for key in ['audio', 'mic_audio', 'system_audio']:
            path = manifest_path.parent / case[key]
            identity = digest(path)
            if identity != case['sha256'][key]:
                raise ValueError(f"{case['id']}: {key} differs from the recorded fixture hash")
            fixture_identities[path] = identity
    process, elapsed, usage = run_measured(
        [str(binary.resolve()), str(model.resolve()), str(manifest_path.resolve()), language, variant])
    if process.returncode:
        raise RuntimeError(f'evaluation failed ({process.returncode}): {process.stderr[-4000:]}')
    for field, path in [('manifest_sha256', manifest_path), ('model_sha256', model), ('runner_sha256', binary)]:
        if digest(path) != identities[field]:
            raise ValueError(f'{field} changed during evaluation; discard this run and retry with stable inputs')
    for path, identity in fixture_identities.items():
        if digest(path) != identity:
            raise ValueError(f'fixture {path.name} changed during evaluation; discard this run')
    output = json.loads(process.stdout)
    predictions = {case['id']: case for case in output['cases']}
    if len(output['cases']) != len(cases) or predictions.keys() != {case['id'] for case in cases}:
        raise ValueError('runner did not return exactly the requested cases')
    scored = []
    for case in cases:
        result = predictions[case['id']]
        speech = [s for s in result['segments'] if s['text'].strip()]
        scored.append({**result, 'reference': case['reference'], 'tags': case['tags'],
                       'words': word_error(case['reference'], result['text']),
                       'speakers': speaker_error(case['turns'], speech)})
    reference_words = sum(c['words']['reference_words'] for c in scored)
    speech_errors = sum(c['words']['errors'] for c in scored if c['words']['reference_words'])
    speaker_ms = sum(c['speakers']['reference_speaker_ms'] for c in scored)
    speaker_errors = sum(c['speakers']['error_ms'] for c in scored)
    return {'schema_version': 1, 'measured_at': datetime.now(timezone.utc).isoformat(),
        'hardware': {'system': platform.platform(), 'machine': platform.machine()},
        **identities, 'scorer_sha256': scorer_fingerprint(),
        'variant': variant,
        'model_language': language, 'provenance': manifest['provenance'],
        'provenance_note': manifest['provenance_note'],
        'measurement_scope': 'Cold final-pass ASR plus production Me/Them channel attribution. '
            'First segment measures final-pass callback latency, not live-preview latency. '
            'Cross-meeting identity and multiple remote speakers are not evaluated. '
            'CPU/RSS cover the whole runner process, not individual clips.',
        'runtime': {'wall_seconds': elapsed, 'user_cpu_seconds': usage.ru_utime,
                    'system_cpu_seconds': usage.ru_stime,
                    'peak_rss_bytes': usage.ru_maxrss * (1 if sys.platform == 'darwin' else 1024)},
        'summary': {'reference_words': reference_words, 'speech_word_errors': speech_errors,
                    'word_error_rate': speech_errors / reference_words if reference_words else None,
                    'false_silence_words': sum(c['words']['false_silence_words'] for c in scored),
                    'speaker_error_rate': speaker_errors / speaker_ms if speaker_ms else None},
        'cases': scored}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--model-language', default='en', help='en for the production English model; auto for multilingual detection')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--compare', type=Path)
    parser.add_argument('--variant', choices=['candidate', 'legacy'], default='candidate',
                        help='legacy runs the frozen pre-silence-guard control; never used by the app')
    args = parser.parse_args()
    report = evaluate(args.binary, args.model, args.manifest, args.model_language, args.variant)
    if args.compare:
        report['comparison'] = compare(json.loads(args.compare.read_text()), report)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False) + '\n')
    print(json.dumps({'report': str(args.output), **report['summary']}, indent=2))

if __name__ == '__main__':
    main()
