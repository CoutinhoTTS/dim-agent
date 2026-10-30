"""A corpus checkout is used only when it is exactly the pinned manifest.

Run: python3 -m unittest discover -s benchmarks -p 'corpus_flow_test.py'
"""

from __future__ import annotations

import hashlib
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent / "runner"))

import corpus  # noqa: E402
from assemble_tasks import digest_dir  # noqa: E402


class CorpusPinFlowTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        root = Path(self._tmp.name)
        self.checkout = root / "data" / "frontier"
        for name in ("alpha", "beta"):
            (self.checkout / name / "tests").mkdir(parents=True)
            (self.checkout / name / "task.toml").write_text(f'[task]\nname = "{name}"\n')
            (self.checkout / name / "tests" / "test.sh").write_text("exit 0\n")
        tasks = {name: {"digest": digest_dir(self.checkout / name), "upstream": "tb",
                        "overlaid": []} for name in ("alpha", "beta")}
        manifest = {"tree_digest": hashlib.sha256(json.dumps(tasks, sort_keys=True).encode()).hexdigest(),
                    "tasks": tasks}
        self.pinned = root / "dim" / "MANIFEST.json"
        self.pinned.parent.mkdir()
        self.pinned.write_text(json.dumps(manifest))
        (self.checkout / "MANIFEST.json").write_text(json.dumps(manifest, indent=2))

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_the_pinned_corpus_passes(self) -> None:
        self.assertEqual(corpus.verify(self.pinned, self.checkout), [])

    def test_an_edited_task_is_refused(self) -> None:
        (self.checkout / "alpha" / "tests" / "test.sh").write_text("exit 1\n")
        self.assertEqual(corpus.verify(self.pinned, self.checkout),
                         ["alpha: content differs from its pinned digest"])

    def test_a_missing_or_extra_task_is_refused(self) -> None:
        shutil.rmtree(self.checkout / "beta")
        shutil.copytree(self.checkout / "alpha", self.checkout / "gamma")
        self.assertEqual(corpus.verify(self.pinned, self.checkout),
                         ["beta: missing", "gamma: not in the pinned manifest"])

    def test_a_missing_checkout_is_refused_without_a_traceback(self) -> None:
        missing = self.checkout.parent / "absent"
        self.assertEqual(corpus.verify(self.pinned, missing), [f"{missing} is not a directory"])
        self.assertIn("cannot read the pinned manifest",
                      corpus.verify(missing / "MANIFEST.json", self.checkout)[0])

    def test_a_corpus_regenerated_without_updating_the_pin_is_refused(self) -> None:
        manifest = json.loads(self.pinned.read_text())
        manifest["tree_digest"] = "0" * 64
        (self.checkout / "MANIFEST.json").write_text(json.dumps(manifest))
        self.assertEqual(corpus.verify(self.pinned, self.checkout),
                         [f"{self.checkout / 'MANIFEST.json'} differs from the pinned {self.pinned}"])


if __name__ == "__main__":
    unittest.main()
