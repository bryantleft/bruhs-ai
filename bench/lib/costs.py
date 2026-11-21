"""Cost per accepted result keeps failed attempts and unknown charges visible.

Billed values are distinct from CLI-reported estimates. This reducer validates
receipt attribution and refuses overlapping inclusive parent/child charges; it
cannot independently attest an invoice or reconstruct unrecorded human effort.
"""

from __future__ import annotations

from decimal import Decimal, InvalidOperation
from typing import Any


def amount(value: object, field: str) -> Decimal | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (str, int, float, Decimal)):
        raise TypeError(f"{field} must be a nonnegative finite number or null")
    try:
        result = Decimal(str(value))
    except InvalidOperation as error:
        raise ValueError(f"Invalid {field}") from error
    if not result.is_finite() or result < 0:
        raise ValueError(f"Invalid {field}")
    return result


def formatted(value: Decimal | None) -> str | None:
    return None if value is None else format(value.normalize(), "f")


def charge_total(
    records: list[dict[str, Any]],
) -> tuple[Decimal | None, Decimal | None]:
    indexed = {record["run_id"]: record for record in records}
    if len(indexed) != len(records):
        raise ValueError("Duplicate charged run identity")
    known: list[Decimal] = []
    complete = bool(records)
    for record in records:
        ancestors: set[str] = {record["run_id"]}
        parent = record.get("parent_run_id")
        while parent is not None:
            if parent in ancestors:
                raise ValueError("Cyclic charge lineage")
            ancestors.add(parent)
            if parent not in indexed:
                raise ValueError("Unknown charge parent")
            ancestor = indexed[parent]
            if ancestor.get("includes_descendants") is True:
                raise ValueError("Inclusive parent and separate child charges overlap")
            parent = ancestor.get("parent_run_id")
        verified = record.get("billing_evidence_verified") is True and bool(
            record.get("billing_evidence")
        )
        for field in ("billed_model_usd", "billed_compute_usd"):
            value = amount(record.get(field), field)
            if value is None or not verified:
                complete = False
            else:
                known.append(value)
    subtotal = sum(known, Decimal(0)) if known else None
    return (subtotal if complete else None), subtotal


def summarize(
    plan: list[dict[str, str]], attempts: list[dict[str, Any]]
) -> dict[str, Any]:
    declared = {cell["id"]: cell for cell in plan}
    if not plan or len(declared) != len(plan):
        raise ValueError("Require distinct predeclared attempt identities")
    indexed = {attempt["id"]: attempt for attempt in attempts}
    if len(indexed) != len(attempts) or not set(indexed).issubset(declared):
        raise ValueError("Duplicate or undeclared attempt")
    rows: list[dict[str, Any]] = []
    for identity, cell in declared.items():
        attempt = indexed.get(identity, {})
        if attempt and attempt.get("entrant") != cell["entrant"]:
            raise ValueError("Entrant attribution changed")
        status = attempt.get("status", "unrecorded")
        strict_success = attempt.get("strict_success", False)
        if type(strict_success) is not bool or (
            strict_success and status != "completed"
        ):
            raise ValueError("Only completed attempts can claim strict success")
        billed, known = charge_total(attempt.get("charges", []))
        complete_lineage = attempt.get("descendant_accounting") == "complete"
        if not complete_lineage:
            billed = None
        seconds = amount(attempt.get("human_seconds"), "human_seconds")
        rate = amount(attempt.get("human_hourly_usd"), "human_hourly_usd")
        human = (
            seconds * rate / Decimal(3600)
            if seconds is not None
            and rate is not None
            and attempt.get("human_time_complete") is True
            else None
        )
        total = billed + human if billed is not None and human is not None else None
        rows.append(
            {
                "id": identity,
                "entrant": cell["entrant"],
                "status": status,
                "strict_success": strict_success,
                "billed_model_and_compute_usd": formatted(billed),
                "known_billed_subtotal_usd": formatted(known),
                "human_usd": formatted(human),
                "total_usd": formatted(total),
                "descendant_accounting_complete": complete_lineage,
                "reported_model_usd": formatted(
                    amount(attempt.get("reported_model_usd"), "reported_model_usd")
                ),
                "reported_usage_note": attempt.get("reported_usage_note"),
                "unknown_components": [
                    name
                    for name, value in (
                        ("billed model/compute or descendant coverage", billed),
                        ("complete human time/rate", human),
                    )
                    if value is None
                ],
            }
        )
    entrants: dict[str, Any] = {}
    for entrant in dict.fromkeys(cell["entrant"] for cell in plan):
        group = [row for row in rows if row["entrant"] == entrant]
        successes = sum(row["strict_success"] for row in group)
        totals = [amount(row["total_usd"], "total_usd") for row in group]
        total = (
            sum((value for value in totals if value is not None), Decimal(0))
            if all(value is not None for value in totals)
            else None
        )
        entrants[entrant] = {
            "planned_attempts": len(group),
            "strict_successes": successes,
            "complete_total_cost_usd": formatted(total),
            "cost_per_strict_success_usd": formatted(
                total / successes if total is not None and successes else None
            ),
            "cost_accounting_complete": total is not None,
        }
    return {
        "currency": "USD",
        "all_planned_attempts_retained": True,
        "attempts": rows,
        "entrants": entrants,
        "limitation": "Billing evidence and lineage assertions are supplied by the operator. Reported model cost is never substituted for billed cost. Missing attempts and unknown amounts prevent a total-cost comparison.",
    }
