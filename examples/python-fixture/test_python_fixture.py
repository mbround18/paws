"""Dependency-free test for python-fixture: runnable with plain `python3 -m unittest`
or directly via `python3 test_python_fixture.py` (no pytest install required).
"""

import unittest

from python_fixture import add


class AddTests(unittest.TestCase):
    def test_add(self):
        self.assertEqual(add(2, 3), 5)

    def test_add_negative(self):
        self.assertEqual(add(-1, 1), 0)


if __name__ == "__main__":
    unittest.main()
