import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from transcription_eval import compare, evaluate, run_measured, speaker_error, word_error
from transcription_release_check import check


def turn(start, end, speaker):
    return {'start_ms': start, 'end_ms': end, 'speaker_id': speaker}


class WordErrorTests(unittest.TestCase):
    def test_insertions_and_deletions_require_sequence_alignment(self):
        self.assertEqual(word_error('one two three', 'one extra two three')['errors'], 1)
        self.assertEqual(word_error('one two three', 'one three')['errors'], 1)

    def test_punctuation_case_and_unicode_normalization(self):
        self.assertEqual(word_error('RÉUNION: Let’s test SQLite.', "réunion let's test sqlite")['errors'], 0)

    def test_silence_has_no_word_error_denominator(self):
        result = word_error('', 'Thank you')
        self.assertIsNone(result['word_error_rate'])
        self.assertEqual(result['false_silence_words'], 2)

    def test_no_hypothesis_is_all_deletions(self):
        self.assertEqual(word_error('the build passed', '')['word_error_rate'], 1)

    def test_decomposed_accents_are_the_same_words(self):
        self.assertEqual(word_error('réunion', 're\u0301union')['errors'], 0)


class SpeakerErrorTests(unittest.TestCase):
    def test_swapped_channel_is_confusion(self):
        self.assertEqual(speaker_error([turn(0, 1000, 'me')], [turn(0, 1000, 'them')])['confusion_ms'], 1000)

    def test_missing_overlap_counts_the_missing_speaker(self):
        result = speaker_error([turn(0, 1000, 'me'), turn(0, 1000, 'them')], [turn(0, 1000, 'me')])
        self.assertEqual(result['reference_speaker_ms'], 2000)
        self.assertEqual(result['missed_ms'], 1000)
        self.assertEqual(result['error_rate'], 0.5)

    def test_false_alarm_outside_reference_is_counted(self):
        result = speaker_error([turn(0, 1000, 'me')], [turn(0, 1500, 'me')])
        self.assertEqual(result['false_alarm_ms'], 500)

    def test_silence_does_not_claim_perfect_attribution(self):
        self.assertIsNone(speaker_error([], [])['error_rate'])

    def test_invalid_interval_fails_instead_of_changing_metrics(self):
        with self.assertRaises(ValueError):
            speaker_error([turn(10, 5, 'me')], [])

    def test_infinite_duration_cannot_turn_an_error_rate_into_nan(self):
        with self.assertRaises(ValueError):
            speaker_error([turn(0, float('inf'), 'me')], [])


class ComparisonTests(unittest.TestCase):
    def test_changed_audio_or_model_cannot_claim_improvement(self):
        baseline = {'manifest_sha256': 'one', 'model_sha256': 'model', 'model_language': 'en', 'cases': []}
        candidate = copy.deepcopy(baseline)
        candidate['manifest_sha256'] = 'two'
        with self.assertRaises(ValueError):
            compare(baseline, candidate)

    def test_changed_scoring_protocol_is_not_a_pipeline_improvement(self):
        baseline = {'manifest_sha256': 'one', 'model_sha256': 'model', 'model_language': 'en',
                    'scorer_sha256': 'old', 'cases': []}
        candidate = {**baseline, 'scorer_sha256': 'new'}
        with self.assertRaises(ValueError):
            compare(baseline, candidate)


class ResourceTests(unittest.TestCase):
    def test_each_child_reports_its_own_peak_rss(self):
        python = str(Path(sys.executable).resolve())
        large = run_measured([python, '-S', '-c', "data = b'x' * (80 * 1024 * 1024)"])[2]
        small = run_measured([python, '-S', '-c', 'pass'])[2]
        self.assertLess(small.ru_maxrss, large.ru_maxrss)


class ProvenanceTests(unittest.TestCase):
    def test_fixture_changes_during_execution_invalidate_the_run(self):
        self.assert_mutation_rejected("(root / 'audio.wav').write_bytes(b'changed')", 'fixture')

    def test_model_changes_during_execution_invalidate_the_run(self):
        self.assert_mutation_rejected("Path(__file__).write_text('changed')", 'model_sha256')

    def assert_mutation_rejected(self, mutation, expected):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'audio.wav').write_bytes(b'synthetic fixture')
            sha = hashlib.sha256(b'synthetic fixture').hexdigest()
            manifest = root / 'manifest.json'
            manifest.write_text(json.dumps({'provenance': 'synthetic', 'provenance_note': 'unit fixture',
                'cases': [{'id': 'one', 'audio': 'audio.wav', 'mic_audio': 'audio.wav',
                    'system_audio': 'audio.wav', 'sha256': {k: sha for k in ['audio', 'mic_audio', 'system_audio']}}]}))
            model = root / 'fake_runner.py'
            model.write_text("from pathlib import Path\nroot = Path(__file__).parent\n" + mutation
                             + "\nprint('{\"cases\": []}')\n")
            with self.assertRaisesRegex(ValueError, expected):
                evaluate(Path(sys.executable), model, manifest, 'en')


class ReleaseEvidenceTests(unittest.TestCase):
    evidence = Path(__file__).resolve().parents[1] / 'bench/transcription/results/2026-11-13-base-en'

    def test_actual_recorded_evidence_meets_its_criteria(self):
        self.assertEqual(check(self.evidence)['status'], 'passed')

    def test_speech_timing_regression_rejects_an_otherwise_good_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'evidence'
            shutil.copytree(self.evidence, root)
            path = root / 'candidate-1.json'
            report = json.loads(path.read_text())
            report['cases'][0]['segments'][0]['end_ms'] += 10
            path.write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, 'timing'):
                check(root)

    def test_unsupported_confidence_rejects_saved_pipeline_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'evidence'
            shutil.copytree(self.evidence, root)
            path = root / 'postprocess.json'
            report = json.loads(path.read_text())
            report['cases'][0]['segments'][0]['confidence'] = 1.0
            path.write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, 'confidence'):
                check(root)

    def test_non_finite_timings_cannot_bypass_resource_comparison(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'evidence'
            shutil.copytree(self.evidence, root)
            for path in root.glob('candidate-*.json'):
                report = json.loads(path.read_text())
                report['runtime']['wall_seconds'] = float('nan')
                path.write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, 'non-finite'):
                check(root)

    def test_different_corpora_cannot_be_aggregated_as_repeated_trials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'evidence'
            shutil.copytree(self.evidence, root)
            for name in ['control-2.json', 'candidate-2.json']:
                path = root / name
                report = json.loads(path.read_text())
                report['manifest_sha256'] = 'different-corpus'
                path.write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, 'manifest_sha256'):
                check(root)

    def test_negative_cpu_cannot_claim_a_performance_improvement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'evidence'
            shutil.copytree(self.evidence, root)
            path = root / 'candidate-1.json'
            report = json.loads(path.read_text())
            report['runtime']['user_cpu_seconds'] = -100
            path.write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, 'nonnegative'):
                check(root)


if __name__ == '__main__':
    unittest.main()
