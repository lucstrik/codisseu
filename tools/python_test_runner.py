#!/usr/bin/env python3
"""Codisseu's explicit pytest/unittest runner. Structured results, no shell."""
import argparse
import inspect
import json
import os
from pathlib import Path
import sys
import time
import traceback
import unittest

MAX_CASES = 10000
MAX_TEXT = 16384
MAX_REPORT = 16 * 1024 * 1024


def text(value):
    value = str(value)
    return value if len(value) <= MAX_TEXT else value[:MAX_TEXT] + "… [truncated]"


def run_pytest(args):
    import pytest

    cases = {}

    class Results:
        def pytest_collection_modifyitems(self, items):
            if len(items) > MAX_CASES:
                raise RuntimeError("Python test collection exceeds 10000 cases")
            for item in items:
                cases[item.nodeid] = {
                    "name": item.nodeid, "title": item.name,
                    "ancestors": item.nodeid.split("::")[:-1],
                    "class": "::".join(item.nodeid.split("::")[1:-1]),
                    "path": str(item.path), "line": item.location[1] + 1,
                    "outcome": "Error", "seconds": 0.0, "details": "Test did not run",
                }

        def pytest_runtest_logreport(self, report):
            case = cases.get(report.nodeid)
            if case is None:
                return
            case["seconds"] += report.duration
            if report.failed:
                case["outcome"] = "Failed" if report.when == "call" else "Error"
                case["details"] = text(report.longrepr)
            elif report.skipped and case["details"] == "Test did not run":
                case["outcome"] = "Skipped"
                case["details"] = text(report.longrepr)
            elif report.passed and report.when == "call":
                case["outcome"] = "Passed"
                case["details"] = ""

        def pytest_collectreport(self, report):
            if report.failed and len(cases) < MAX_CASES:
                cases[report.nodeid] = {
                    "name": report.nodeid, "title": "Collection error: " + report.nodeid,
                    "class": "", "ancestors": [], "path": None, "line": None,
                    "outcome": "Error", "seconds": 0.0, "details": text(report.longrepr),
                }

    targets = args.selected or ([args.file] if args.file else [str(args.root)])
    code = pytest.main(["--color=no", "-p", "no:cacheprovider", "-q", "--rootdir", str(args.root)] + targets, plugins=[Results()])
    return int(code), list(cases.values())


def run_unittest(args):
    cases = {}

    class Results(unittest.TextTestResult):
        def startTest(self, test):
            super().startTest(test)
            if len(cases) >= MAX_CASES:
                raise RuntimeError("Python test collection exceeds 10000 cases")
            method = getattr(test, getattr(test, "_testMethodName", ""), None)
            try:
                # unittest.skip and other wraps-preserving decorators point at
                # a wrapper; source navigation belongs to the original test.
                method = inspect.unwrap(method)
                path = inspect.getsourcefile(method)
                line = inspect.getsourcelines(method)[1]
            except (TypeError, OSError, ValueError):
                path, line = None, None
            cases[test.id()] = {
                "name": test.id(), "title": getattr(test, "_testMethodName", test.id()),
                "class": test.__class__.__module__ + "." + test.__class__.__qualname__,
                "ancestors": test.id().split(".")[:-1], "path": path, "line": line,
                "outcome": "Error", "seconds": 0.0, "details": "Test did not run",
            }
            test._codisseu_start = time.perf_counter()

        def stopTest(self, test):
            cases[test.id()]["seconds"] = time.perf_counter() - test._codisseu_start
            super().stopTest(test)

        def record(self, test, outcome, details=""):
            test = getattr(test, "test_case", test)
            if test.id() not in cases:
                if len(cases) >= MAX_CASES:
                    raise RuntimeError("Python test report exceeds 10000 cases")
                cases[test.id()] = {"name": test.id(), "title": str(test), "class": "", "ancestors": [], "path": None, "line": None, "outcome": "Error", "seconds": 0.0, "details": "Test did not run"}
            case = cases[test.id()]
            if outcome == "Passed" and case["details"] != "Test did not run":
                return
            case["outcome"], case["details"] = outcome, text(details)

        def addSuccess(self, test):
            super().addSuccess(test)
            self.record(test, "Passed")

        def addFailure(self, test, err):
            super().addFailure(test, err)
            self.record(test, "Failed", self._exc_info_to_string(err, test))

        def addError(self, test, err):
            super().addError(test, err)
            self.record(test, "Error", self._exc_info_to_string(err, test))

        def addSkip(self, test, reason):
            super().addSkip(test, reason)
            self.record(test, "Skipped", reason)

        def addExpectedFailure(self, test, err):
            super().addExpectedFailure(test, err)
            self.record(test, "Skipped", "Expected failure: " + self._exc_info_to_string(err, test))

        def addUnexpectedSuccess(self, test):
            super().addUnexpectedSuccess(test)
            self.record(test, "Failed", "Unexpected success")

        def addSubTest(self, test, subtest, err):
            super().addSubTest(test, subtest, err)
            if err is not None:
                outcome = "Failed" if issubclass(err[0], test.failureException) else "Error"
                self.record(test, outcome, str(subtest) + "\n" + self._exc_info_to_string(err, test))

    loader = unittest.TestLoader()
    if args.selected:
        suite = loader.loadTestsFromNames(args.selected)
    elif args.file:
        file = Path(args.file).resolve()
        module = file.relative_to(args.root).with_suffix("").as_posix().replace("/", ".")
        suite = loader.loadTestsFromName(module)
    else:
        start = args.root / "tests" if (args.root / "tests").is_dir() else args.root
        suite = loader.discover(str(start), pattern="test*.py")
    result = unittest.TextTestRunner(stream=sys.stdout, verbosity=2, resultclass=Results).run(suite)
    return 0 if result.wasSuccessful() else 1, list(cases.values())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--framework", choices=["pytest", "unittest"], required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--file")
    parser.add_argument("--selected", action="append", default=[])
    args = parser.parse_args()
    args.root = args.root.resolve(strict=True)
    os.chdir(args.root)
    sys.path.insert(0, str(args.root))
    if (args.root / "tests").is_dir():
        sys.path.insert(0, str(args.root / "tests"))
    sys.dont_write_bytecode = True
    error = None
    try:
        code, cases = run_pytest(args) if args.framework == "pytest" else run_unittest(args)
    except BaseException:
        code, cases, error = 2, [], text(traceback.format_exc())
    payload = {"schema": 1, "framework": args.framework, "exit_code": code, "cases": cases, "error": error}
    encoded = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    if len(encoded) > MAX_REPORT:
        encoded = json.dumps({"schema": 1, "framework": args.framework, "exit_code": 2, "cases": [], "error": "Structured Python test report exceeds 16 MiB"}).encode()
        code = 2
    args.report.write_bytes(encoded)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
