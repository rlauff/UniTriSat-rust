"""Plotting helper validation and subprocess failure tests."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from render import validate_tetrahedra


class DisplayInputTests(unittest.TestCase):
    def test_fractional_coordinates_are_preserved(self):
        tetrahedra = [[[0.0, 0.0, 0.0], [1.25, 0.0, 0.0],
                      [0.0, 2.5, 0.0], [0.0, 0.0, 3.75]]]
        self.assertIs(validate_tetrahedra(tetrahedra), tetrahedra)
        self.assertEqual(tetrahedra[0][1][0], 1.25)

    def test_bad_shapes_fail(self):
        for data in (None, [], [None], [[[0, 0, 0]]], [[[0, 0]] * 4]):
            with self.subTest(data=data), self.assertRaises(ValueError):
                validate_tetrahedra(data)

    def test_nonfinite_and_nonnumeric_coordinates_fail(self):
        for coordinate in (True, None, "1.0", float("nan"), float("inf")):
            with self.subTest(coordinate=coordinate), self.assertRaises(ValueError):
                validate_tetrahedra([[[coordinate, 0, 0]] * 4])

    def test_invalid_input_child_exits_nonzero_without_loading_plot_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.json"
            path.write_text(json.dumps([[[0, 0, 0]]]), encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(Path(__file__).with_name("render.py")), str(path)],
                capture_output=True, text=True, check=False,
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("four vertices", result.stderr)


if __name__ == "__main__":
    unittest.main()
