"""Fail-closed tests for the sanitizer log replay path; no native build needed."""
import pathlib
import subprocess
import tempfile
import unittest

RUNNER = pathlib.Path(__file__).with_name("run.sh")
CLEAN = "wrong results: 0\n"
RACE = (
    "WARNING: ThreadSanitizer: data race (pid=123)\n"
    "SUMMARY: ThreadSanitizer: data race /tmp/pristine-vendor/cddlib/lib-src/cddlp.c:3677 in dd_LPSolve\n"
    + CLEAN
)


class GateTests(unittest.TestCase):
    def check(self, variant, status, log, accepted):
        with tempfile.NamedTemporaryFile(mode="w") as output:
            output.write(log)
            output.flush()
            result = subprocess.run(
                ["bash", str(RUNNER), "--check-output", variant, str(status), output.name],
                capture_output=True,
                text=True,
                check=False,
            )
        self.assertEqual(result.returncode == 0, accepted, result.stderr)

    def test_patched(self):
        self.check("patched", 0, CLEAN, True)
        for status, log in [
            (66, RACE), (1, CLEAN), (0, ""), (0, CLEAN + CLEAN),
            (0, "wrong results: 1\n"), (0, "wrong results: 00\n"),
            (0, "FATAL: ThreadSanitizer: unexpected memory mapping\n" + CLEAN),
            (0, RACE), (0, "ERROR: ThreadSanitizer: startup failure\n" + CLEAN),
        ]:
            with self.subTest(status=status, log=log):
                self.check("patched", status, log, False)

    def test_positive_control(self):
        self.check("pristine", 66, RACE, True)
        self.check("pristine", 66, RACE.replace("cddlp.c", "cddlp_f.c"), True)
        for status, log in [
            (0, CLEAN), (0, RACE), (1, RACE), (66, CLEAN),
            (66, RACE.replace("cddlp.c", "other.c")),
            (66, RACE.replace(CLEAN, "")), (66, RACE + CLEAN),
            (66, RACE.replace(CLEAN, "wrong results: 2\n")),
            (66, RACE + "WARNING: ThreadSanitizer: thread leak (pid=123)\n"),
            (66, RACE + "FATAL: ThreadSanitizer: crashed\n"),
        ]:
            with self.subTest(status=status, log=log):
                self.check("pristine", status, log, False)

    def test_bad_arguments(self):
        self.check("unknown", 0, CLEAN, False)
        self.check("patched", "bad", CLEAN, False)


if __name__ == "__main__":
    unittest.main()
