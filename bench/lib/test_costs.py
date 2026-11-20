"""Cost-accounting failures must not become zero-cost successes."""

from __future__ import annotations

import unittest

from costs import amount, summarize


def cell(identity="one", entrant="A"):
    return {"id": identity, "entrant": entrant}


def receipt(identity="one", success=True):
    return {
        "id": identity,
        "entrant": "A",
        "status": "completed" if success else "failed",
        "strict_success": success,
        "descendant_accounting": "complete",
        "human_time_complete": True,
        "human_seconds": 3600,
        "human_hourly_usd": 10,
        "charges": [
            {
                "run_id": "root",
                "parent_run_id": None,
                "billed_model_usd": 3,
                "billed_compute_usd": "0.5",
                "billing_evidence_verified": True,
                "billing_evidence": "owned invoice attribution receipt",
            }
        ],
    }


class Costs(unittest.TestCase):
    def test_failures_stay_in_cost_per_success_denominator(self):
        first, second = receipt(), receipt("two", False)
        result = summarize([cell(), cell("two")], [first, second])["entrants"]["A"]
        self.assertEqual(result["complete_total_cost_usd"], "27")
        self.assertEqual(result["cost_per_strict_success_usd"], "27")

    def test_missing_attempt_cannot_disappear_or_be_free(self):
        result = summarize([cell(), cell("two")], [receipt()])
        self.assertEqual(len(result["attempts"]), 2)
        self.assertEqual(result["attempts"][1]["status"], "unrecorded")
        self.assertIsNone(result["entrants"]["A"]["complete_total_cost_usd"])

    def test_reported_cost_does_not_substitute_for_billing(self):
        row = receipt()
        row["charges"] = []
        row["reported_model_usd"] = "0.02"
        result = summarize([cell()], [row])["attempts"][0]
        self.assertEqual(result["reported_model_usd"], "0.02")
        self.assertIsNone(result["total_usd"])
        self.assertIsNone(result["known_billed_subtotal_usd"])

    def test_invoice_required_for_billed_values(self):
        row = receipt()
        row["charges"][0]["billing_evidence_verified"] = False
        self.assertIsNone(summarize([cell()], [row])["attempts"][0]["total_usd"])

    def test_unobserved_descendants_block_complete_cost(self):
        row = receipt()
        row["descendant_accounting"] = "unknown"
        self.assertIsNone(summarize([cell()], [row])["attempts"][0]["total_usd"])

    def test_inclusive_parent_and_separate_child_are_rejected(self):
        row = receipt()
        row["charges"][0]["includes_descendants"] = True
        child = dict(row["charges"][0], run_id="child", parent_run_id="root")
        row["charges"].append(child)
        with self.assertRaisesRegex(ValueError, "overlap"):
            summarize([cell()], [row])

    def test_cycles_and_unknown_parents_are_rejected(self):
        for parent in ["root", "absent"]:
            row = receipt()
            row["charges"][0]["parent_run_id"] = parent
            with self.assertRaises(ValueError):
                summarize([cell()], [row])

    def test_negative_nonfinite_and_boolean_amounts_are_rejected(self):
        for value in [-1, "NaN", "Infinity", True, {}]:
            with self.assertRaises((ValueError, TypeError)):
                amount(value, "fixture")

    def test_duplicate_and_undeclared_attempts_are_rejected(self):
        for plan, rows in [
            ([cell(), cell()], [receipt()]),
            ([cell()], [receipt(), receipt()]),
            ([cell()], [receipt("other")]),
        ]:
            with self.assertRaises(ValueError):
                summarize(plan, rows)

    def test_zero_cost_requires_complete_explicit_evidence(self):
        row = receipt()
        row["human_seconds"] = 0
        row["charges"][0].update(billed_model_usd=0, billed_compute_usd=0)
        self.assertEqual(summarize([cell()], [row])["attempts"][0]["total_usd"], "0")

    def test_no_success_keeps_cost_per_success_unknown(self):
        row = receipt(success=False)
        self.assertIsNone(
            summarize([cell()], [row])["entrants"]["A"]["cost_per_strict_success_usd"]
        )

    def test_failed_status_cannot_claim_strict_success(self):
        row = receipt()
        row["status"] = "failed"
        with self.assertRaises(ValueError):
            summarize([cell()], [row])


if __name__ == "__main__":
    unittest.main()
