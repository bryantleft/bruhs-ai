#!/usr/bin/env python3
"""Generate public, synthetic meeting-audio fixtures using installed macOS voices."""
import argparse
import array
import hashlib
import json
import math
from pathlib import Path
import random
import subprocess
import sys
import wave

RATE = 16000

def write_audio(path, samples):
    pcm = array.array('h', (max(-32768, min(32767, round(s))) for s in samples))
    if sys.byteorder != 'little':
        pcm.byteswap()
    with wave.open(str(path), 'wb') as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(RATE)
        output.writeframes(pcm.tobytes())

def speak(root, name, voice, text):
    text_file = root / f'{name}.txt'
    text_file.write_text(text)
    path = root / f'{name}.wav'
    subprocess.run(['say', '-v', voice, '-o', str(path), '--file-format=WAVE',
                    '--data-format=LEI16@16000', '-f', str(text_file)], check=True)
    with wave.open(str(path), 'rb') as source:
        if (source.getnchannels(), source.getsampwidth(), source.getframerate()) != (1, 2, RATE):
            raise ValueError(f'{name}: expected mono 16 kHz PCM16')
        samples = array.array('h', source.readframes(source.getnframes()))
    if sys.byteorder != 'little':
        samples.byteswap()
    return list(samples)

def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    cases = []
    def add(name, mic, system, reference, tags, language='en', turns=None):
        length = max(len(mic), len(system))
        mic = list(mic) + [0] * (length - len(mic))
        system = list(system) + [0] * (length - len(system))
        # Match capture's equal-weight sum with PCM16 clipping.
        mixed = [a + b for a, b in zip(mic, system)]
        files = {'audio': f'{name}.wav', 'mic_audio': f'{name}.mic.wav',
                 'system_audio': f'{name}.sys.wav'}
        for key, data in [('audio', mixed), ('mic_audio', mic), ('system_audio', system)]:
            write_audio(root / files[key], data)
        if turns is None:
            source = mic if any(mic) else system
            nonzero = [i for i, value in enumerate(source) if value]
            turns = [] if not nonzero else [{'start_ms': nonzero[0] * 1000 / RATE,
                'end_ms': (nonzero[-1] + 1) * 1000 / RATE,
                'speaker_id': 'me' if any(mic) else 'them'}]
        cases.append({'id': name, **files, 'reference': reference, 'tags': tags,
                      'language': language, 'turns': turns})
    lines = [
        ('clean', 'Samantha', 'The deployment passed its tests. We can release the update tomorrow.', ['clean', 'mic']),
        ('technical', 'Daniel', 'Bryant reviewed the PostgreSQL migration. The API returns a JSON response after authentication.', ['technical_names', 'british_english', 'system']),
        ('accent', 'Aman', 'Please review the pull request before merging. The database backup completed successfully.', ['indian_english', 'system']),
        ('spanish', 'Mónica', 'La reunión comienza mañana. Necesitamos revisar las pruebas antes de publicar la nueva versión.', ['multilingual', 'spanish', 'mic']),
        ('french', 'Amélie', 'La réunion commence demain. Nous devons vérifier les tests avant de publier la nouvelle version.', ['multilingual', 'french', 'system']),
    ]
    originals = {}
    for name, voice, text, tags in lines:
        samples = speak(root, name + '-source', voice, text)
        originals[name] = samples
        add(name, samples if 'mic' in tags else [], samples if 'system' in tags else [], text, tags,
            'es' if name == 'spanish' else 'fr' if name == 'french' else 'en')
    clean = originals['clean']
    rms = math.sqrt(sum(x * x for x in clean) / len(clean))
    rng = random.Random(42)
    sigma = rms / (10 ** (10 / 20))
    noisy = [sample + rng.gauss(0, sigma) for sample in clean]
    add('noise-10db', noisy, [], lines[0][2], ['noise', 'snr_10db', 'mic'])
    first, second = originals['clean'], originals['technical']
    for name, offset in [('turns', len(first) + RATE // 2), ('overlap', len(first) // 2)]:
        turn_ref = [{'start_ms': 0, 'end_ms': len(first) * 1000 / RATE, 'speaker_id': 'me'},
                    {'start_ms': offset * 1000 / RATE,
                     'end_ms': (offset + len(second)) * 1000 / RATE, 'speaker_id': 'them'}]
        add(name, first, [0] * offset + second, lines[0][2] + ' ' + lines[1][2],
            ['channel_turns', 'overlap' if name == 'overlap' else 'sequential'], turns=turn_ref)
    for seconds in [8, 30]:
        add(f'silence-{seconds}s', [0] * (RATE * seconds), [], '', ['digital_silence'])
    for case in cases:
        case['sha256'] = {}
        for key in ['audio', 'mic_audio', 'system_audio']:
            with (root / case[key]).open('rb') as source:
                case['sha256'][key] = hashlib.file_digest(source, 'sha256').hexdigest()
    manifest = {'schema_version': 1, 'provenance': 'synthetic',
        'provenance_note': 'Generated locally with installed macOS say voices; no user recordings or voice clones. '
            'Gold speaker intervals are utterance extents, including internal pauses. Channel attribution only; '
            'this corpus does not validate cross-meeting identity or multiple system-audio speakers. '
            'Overlapping speech uses the scripted utterance order for word-error scoring.',
        'generator': 'scripts/transcription_fixtures.py', 'cases': cases}
    path = root / 'manifest.json'
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
    return path

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(generate(args.output.resolve()))

if __name__ == '__main__':
    main()
