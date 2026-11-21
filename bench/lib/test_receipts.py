"""Frozen unittest inputs and complete-execution accounting for both graders.

Submitted Python shares the tests' interpreter. Receipts detect missing or
skipped checks; they do not attest against arbitrary in-process tampering.
"""

from __future__ import annotations

import ast
import json
import shutil
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class FrozenTests:
    files: tuple[tuple[Path, bytes], ...]
    expected: tuple[str, ...]

    @classmethod
    def read(cls, fixture: Path) -> FrozenTests:
        hidden = fixture / "hidden"
        if not hidden.is_dir():
            raise ValueError(
                "Frozen fixture has no hidden test directory. Restore its declared grader inputs."
            )
        files: dict[Path, bytes] = {}
        for source in sorted(hidden.rglob("*")):
            if "__pycache__" in source.parts or source.name == ".DS_Store":
                continue
            if source.is_symlink():
                raise ValueError(
                    f"Frozen grader input is a symlink: {source}. Use regular fixture files."
                )
            if source.is_file():
                files[source.relative_to(hidden)] = source.read_bytes()
        for source in sorted((fixture / "repo/tests").glob("test_*.py")):
            relative = Path(source.name)
            if relative in files:
                raise ValueError(
                    "Frozen visible and hidden test filenames collide; give each test module a unique name."
                )
            if source.is_symlink():
                raise ValueError(
                    f"Frozen grader input is a symlink: {source}. Use regular fixture files."
                )
            files[relative] = source.read_bytes()
        files.setdefault(Path("__init__.py"), b"")
        expected = []
        for relative, content in files.items():
            if relative.suffix != ".py" or not relative.name.startswith("test_"):
                continue
            module = ".".join(("_bench_hidden", *relative.with_suffix("").parts))
            tree = ast.parse(content, filename=str(relative))
            # Current fixtures declare literal top-level unittest methods. A
            # dynamic/inherited layout must add an explicit inventory contract;
            # it cannot redefine expected coverage by importing the submission.
            for node in tree.body:
                if isinstance(node, ast.ClassDef):
                    for method in node.body:
                        if isinstance(
                            method, (ast.FunctionDef, ast.AsyncFunctionDef)
                        ) and method.name.startswith("test_"):
                            expected.append(f"{module}.{node.name}.{method.name}")
        if len(expected) != len(set(expected)):
            raise ValueError(
                "Frozen fixture repeats a test identity; use distinct class and method names."
            )
        return cls(tuple(sorted(files.items())), tuple(sorted(expected)))

    def install(self, work: Path) -> None:
        destination = work / "_bench_hidden"
        if destination.is_symlink() or destination.is_file():
            destination.unlink()
        elif destination.exists():
            shutil.rmtree(destination)
        destination.mkdir(parents=True)
        for relative, content in self.files:
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)


@dataclass(frozen=True)
class CheckedResult:
    process_exit: int | None
    expected: tuple[str, ...]
    receipt: object
    errors: tuple[str, ...]

    @property
    def passed(self) -> bool:
        return not self.errors

    def to_dict(self) -> dict[str, object]:
        return {
            "passed": self.passed,
            "process_exit": self.process_exit,
            "expected_tests": list(self.expected),
            "receipt": self.receipt,
            "errors": list(self.errors),
            "scope": "Complete unskipped fixture execution, not adversarial in-process attestation.",
        }


def check_receipt(
    status: int | None, raw: bytes, expected: tuple[str, ...]
) -> CheckedResult:
    try:
        receipt = json.loads(raw)
    except (ValueError, UnicodeError):
        receipt = None
    errors = []
    if not expected:
        errors.append(
            "Frozen fixture declares no independent tests. Add literal unittest checks before grading."
        )
    else:
        if status != 0:
            errors.append(
                f"Grader process exited with {status}; inspect the independent check log."
            )
        if not isinstance(receipt, dict):
            errors.append(
                "Grader produced no valid completion receipt. Inspect its log; checks may have exited before completion."
            )
        else:
            if type(receipt.get("tests_run")) is not int or receipt["tests_run"] != len(
                expected
            ):
                errors.append(
                    "Grader test count does not match the frozen fixture inventory."
                )
            for field in ("started", "finished", "succeeded"):
                observed = receipt.get(field)
                if (
                    not isinstance(observed, list)
                    or not all(isinstance(item, str) for item in observed)
                    or sorted(observed) != list(expected)
                ):
                    errors.append(
                        f"Grader {field} test identities do not match the frozen fixture inventory."
                    )
            for field in (
                "failures",
                "errors",
                "skipped",
                "expected_failures",
                "unexpected_successes",
            ):
                if type(receipt.get(field)) is not int or receipt[field] != 0:
                    errors.append(
                        f"Grader reported {field}; every independent check must pass without skips or expected failures."
                    )
    return CheckedResult(status, expected, receipt, tuple(errors))


def result_path(output: Path) -> Path:
    return Path(str(output) + ".result.json")


def receipt_path(output: Path) -> Path:
    return Path(str(output) + ".receipt.json")


def reserve_outputs(output: Path) -> None:
    for path in (result_path(output), receipt_path(output)):
        if path.exists() or path.is_symlink():
            raise ValueError(
                f"Grading artifact already exists: {path}. Choose a fresh output path to retain every attempt."
            )


def save_result(output: Path, raw: bytes, result: CheckedResult) -> None:
    with receipt_path(output).open("xb") as stream:
        stream.write(raw)
    with result_path(output).open("x", encoding="utf-8") as stream:
        json.dump(result.to_dict(), stream, indent=2)
        stream.write("\n")


def verify_result(fixture: Path, output: Path) -> CheckedResult:
    try:
        stored = json.loads(result_path(output).read_bytes())
        raw = receipt_path(output).read_bytes()
    except (OSError, ValueError, UnicodeError) as error:
        raise ValueError(
            "Coding check did not retain valid completion artifacts. Use the shared run_hidden_tests checker and a fresh output path."
        ) from error
    if not isinstance(stored, dict) or type(stored.get("process_exit")) is not int:
        raise ValueError(
            "Coding check has no settled process exit in its completion artifacts."
        )
    result = check_receipt(
        stored["process_exit"], raw, FrozenTests.read(fixture).expected
    )
    if stored != result.to_dict():
        raise ValueError(
            "Stored coding-check result does not match its raw receipt and frozen test inventory."
        )
    return result
