"""Guard the live test's claims without spending provider tokens."""
from pathlib import Path
import runpy
import unittest

validate = runpy.run_path(str(Path(__file__).with_name("test-pith-agent.py")))["validate_tool_call"]
ROOT = Path("/Users/test/disposable-fixture")
TARGET = ROOT / "project" / "target"


class PithAgentHarnessTests(unittest.TestCase):
    def call(self, tool, arguments):
        return validate({"tool": f"mcp__pith__{tool}", "input": arguments}, ROOT, TARGET)

    def test_overlap_must_be_submitted_not_inferred_from_one_counted_file(self):
        with self.assertRaises(AssertionError):
            self.call("pith_estimate_cleanup", {"paths": [str(TARGET)]})
        self.call("pith_estimate_cleanup", {"paths": [str(TARGET), str(TARGET / "build.bin")]})

    def test_scans_and_inspection_cannot_read_unrelated_home_metadata(self):
        for tool in ["pith_scan", "pith_inspect_path"]:
            with self.assertRaises(AssertionError):
                self.call(tool, {"path": "/Users/test/Documents"})
        self.call("pith_scan", {"path": str(ROOT)})
        self.call("pith_inspect_path", {"path": str(TARGET)})
        self.call("pith_inspect_path", {"path": "/"})

    def test_shell_cannot_replace_pith_evidence(self):
        with self.assertRaises(AssertionError):
            validate({"tool": "Bash", "input": {"command": "du"}}, ROOT, TARGET)

    def test_discovery_can_select_only_pith_tools(self):
        self.assertIsNone(validate({"tool": "ToolSearch", "input": {"query": "select:mcp__pith__pith_status,mcp__pith__pith_scan"}}, ROOT, TARGET))
        for query in ["pith", "select:Read", "select:mcp__pith__pith_scan,Bash", "select:"]:
            with self.assertRaises(AssertionError):
                validate({"tool": "ToolSearch", "input": {"query": query}}, ROOT, TARGET)

    def test_candidates_require_the_fixture_scans_exact_id(self):
        event = {"tool": "mcp__pith__pith_cleanup_candidates", "input": {}}
        scan_id = "scan_" + "a" * 32
        with self.assertRaises(AssertionError):
            validate(event, ROOT, TARGET)
        with self.assertRaises(AssertionError):
            validate(event, ROOT, TARGET, scan_id)
        event["input"]["scan_id"] = "scan_" + "b" * 32
        with self.assertRaises(AssertionError):
            validate(event, ROOT, TARGET, scan_id)
        event["input"]["scan_id"] = scan_id
        validate(event, ROOT, TARGET, scan_id)


if __name__ == "__main__":
    unittest.main()
