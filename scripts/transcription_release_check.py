#!/usr/bin/env python3
"""Check the measured transcription release criteria on paired replay evidence."""
import argparse
import json
import math
from pathlib import Path
import statistics

from transcription_eval import compare


def projection(segments):
    return [(item['start_ms'], item['end_ms'], item['text'], item['speaker_id'])
            for item in segments]


def finite_numbers(value):
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError('evidence contains a non-finite measurement')
    if isinstance(value, dict):
        for item in value.values():
            finite_numbers(item)
    elif isinstance(value, list):
        for item in value:
            finite_numbers(item)


def nonnegative_measurements(report):
    for field, value in report['runtime'].items():
        if not isinstance(value, (int, float)) or value < 0:
            raise ValueError(f'invalid nonnegative resource measurement: {field}')


def check(directory):
    controls = sorted(directory.glob('control-*.json'))
    candidates = sorted(directory.glob('candidate-*.json'))
    if not controls or len(controls) != len(candidates):
        raise ValueError('expected a control for every candidate trial')
    old_reports, new_reports = [], []
    for control_path in controls:
        candidate_path = directory / control_path.name.replace('control-', 'candidate-', 1)
        old, new = json.loads(control_path.read_text()), json.loads(candidate_path.read_text())
        finite_numbers(old)
        finite_numbers(new)
        nonnegative_measurements(old)
        nonnegative_measurements(new)
        compare(old, new)  # Reject differing corpus/model/language/scorer identities.
        if old_reports:
            compare(old_reports[0], old)
        if old['variant'] != 'legacy' or new['variant'] != 'candidate':
            raise ValueError('paired reports must identify their inference variants')
        expected = {case['id']: case for case in old['cases']}
        for case in new['cases']:
            baseline = expected[case['id']]
            if baseline['words']['reference_words']:
                if projection(case['segments']) != projection(baseline['segments']):
                    raise ValueError(f'speech text, timing or attribution regressed: {case["id"]}')
            elif case['words']['false_silence_words']:
                raise ValueError(f'false words on silence: {case["id"]}')
        for field in ['word_error_rate', 'speaker_error_rate']:
            if new['summary'][field] > old['summary'][field]:
                raise ValueError(f'quality exceeded the paired baseline: {field}')
        if new['summary']['false_silence_words'] >= old['summary']['false_silence_words']:
            raise ValueError('silence false-word improvement was not demonstrated')
        old_reports.append(old)
        new_reports.append(new)
    for field in ['wall_seconds', 'peak_rss_bytes']:
        if statistics.median(report['runtime'][field] for report in new_reports) > statistics.median(
                report['runtime'][field] for report in old_reports):
            raise ValueError(f'resource median exceeded measured control: {field}')
    def cpu(report):
        return report['runtime']['user_cpu_seconds'] + report['runtime']['system_cpu_seconds']
    if statistics.median(map(cpu, new_reports)) > statistics.median(map(cpu, old_reports)):
        raise ValueError('CPU median exceeded measured control')
    pipeline = json.loads((directory / 'postprocess.json').read_text())
    finite_numbers(pipeline)
    nonnegative_measurements(pipeline)
    reference = new_reports[0]
    for field in ['manifest_sha256', 'model_sha256', 'scorer_sha256']:
        if pipeline[field] != reference[field]:
            raise ValueError(f'finalization evidence has different {field}')
    expected = {case['id']: case for case in reference['cases']}
    if {case['id'] for case in pipeline['cases']} != expected.keys():
        raise ValueError('finalization evidence has different cases')
    for case in pipeline['cases']:
        if projection(case['segments']) != projection(expected[case['id']]['segments']):
            raise ValueError(f'persisted transcript differs from candidate: {case["id"]}')
        first, final = case['first_segment_ms'], case['finalization_ms']
        if final <= 0 or (first is not None and not 0 <= first <= final):
            raise ValueError('invalid first-text/finalization timing')
        if bool(case['segments']) != (first is not None):
            raise ValueError('segment delivery and persisted transcript disagree')
        if any(segment['confidence'] is not None for segment in case['segments']):
            raise ValueError('unsupported confidence estimate in persisted transcript')
    return {'paired_trials': len(controls), 'status': 'passed',
            'criteria': 'No speech text/timing/channel changes; zero silence words; improved silence; no worse corpus WER/channel error or median process wall/CPU/RSS than paired control. Durable pipeline output matches candidate and emits completion.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('results', type=Path)
    args = parser.parse_args()
    print(json.dumps(check(args.results), indent=2))


if __name__ == '__main__':
    main()
