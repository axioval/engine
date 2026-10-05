#!/usr/bin/env python3
"""Self-test of `parity_models.py`: the manifest check refuses every model it
should, and a fetch keeps only a download of the pinned digest. Network-free."""

from __future__ import annotations

import contextlib
import hashlib
import io
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import parity_models

BODY = b"ISO-10303-21;\nEND-ISO-10303-21;\n"
COMMIT = "0123456789abcdef0123456789abcdef01234567"


def model(**changes: str) -> dict:
    entry = {
        "name": "sample.ifc",
        "url": f"https://raw.githubusercontent.com/org/repo/{COMMIT}/dir/sample.ifc",
        "sha256": hashlib.sha256(BODY).hexdigest(),
        "licence": "CC-BY-4.0",
        "attribution": "Example sample",
    }
    entry.update(changes)
    return entry


class Response(io.BytesIO):
    def __enter__(self) -> "Response":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


class ManifestTest(unittest.TestCase):
    def refused(self, models: list[dict]) -> None:
        with self.assertRaises(SystemExit):
            parity_models.check(models)

    def test_the_tracked_manifest_is_valid(self) -> None:
        parity_models.check(parity_models.load())

    def test_a_valid_model_passes(self) -> None:
        parity_models.check([model()])

    def test_an_unpinned_url_is_refused(self) -> None:
        self.refused([model(url="https://raw.githubusercontent.com/org/repo/main/sample.ifc")])
        self.refused([model(url=f"http://raw.githubusercontent.com/org/repo/{COMMIT}/sample.ifc")])

    def test_a_malformed_digest_is_refused(self) -> None:
        self.refused([model(sha256="ABC")])

    def test_an_unlisted_licence_or_missing_attribution_is_refused(self) -> None:
        self.refused([model(licence="CC-BY-ND-4.0")])
        self.refused([model(attribution="")])

    def test_names_are_unique_lowercase_ifc_files(self) -> None:
        self.refused([model(), model()])
        self.refused([model(name="../sample.ifc")])
        self.refused([model(name="Sample.IFC")])


class FetchTest(unittest.TestCase):
    def fetch(self, directory: Path, body: bytes) -> None:
        with mock.patch.object(parity_models.urllib.request, "urlopen", return_value=Response(body)):
            with contextlib.redirect_stdout(io.StringIO()):
                parity_models.fetch([model()], directory)

    def test_a_pinned_download_is_kept(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.fetch(Path(directory), BODY)
            self.assertEqual((Path(directory) / "sample.ifc").read_bytes(), BODY)

    def test_another_digest_is_refused_and_not_kept(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(SystemExit):
                self.fetch(Path(directory), BODY + b"changed")
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_a_stale_file_is_replaced(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "sample.ifc").write_bytes(b"stale")
            self.fetch(Path(directory), BODY)
            self.assertEqual((Path(directory) / "sample.ifc").read_bytes(), BODY)


if __name__ == "__main__":
    unittest.main()
