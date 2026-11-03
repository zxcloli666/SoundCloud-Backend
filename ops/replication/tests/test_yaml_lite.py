import os
import unittest

import support
import yaml_lite


class YamlLite(unittest.TestCase):
    def test_block_and_flow(self):
        text = (
            "a: 1\n"
            "b:\n"
            "  c: {x: [1, 2], y: 'q: #1', z: \"s\\\"t\"}  # trailing\n"
            "  d:\n"
            "    - one\n"
            "    - [two, three]\n"
            "  e: null\n"
            "  f: true\n"
            "g:\n"
        )
        self.assertEqual(
            yaml_lite.loads(text),
            {"a": 1, "b": {"c": {"x": [1, 2], "y": "q: #1", "z": 's"t'}, "d": ["one", ["two", "three"]], "e": None, "f": True}, "g": None},
        )

    def test_duplicate_key_fails(self):
        with self.assertRaises(yaml_lite.YamlError):
            yaml_lite.loads("a: 1\na: 2\n")
        with self.assertRaises(yaml_lite.YamlError):
            yaml_lite.loads("a: {x: 1, x: 2}\n")

    def test_tab_indent_fails(self):
        with self.assertRaises(yaml_lite.YamlError):
            yaml_lite.loads("a:\n\tb: 1\n")

    def test_bad_indent_fails(self):
        with self.assertRaises(yaml_lite.YamlError):
            yaml_lite.loads("a:\n    b: 1\n  c: 2\n")

    def test_missing_space_after_colon_fails(self):
        with self.assertRaises(yaml_lite.YamlError):
            yaml_lite.loads("a:b\n")

    def test_same_as_pyyaml(self):
        try:
            import yaml
        except ImportError:
            self.skipTest("PyYAML not installed")
        path = os.path.join(support.REPL, "ownership.yaml")
        with open(path, encoding="utf-8") as f:
            self.assertEqual(yaml_lite.load(path), yaml.safe_load(f))


if __name__ == "__main__":
    unittest.main()
