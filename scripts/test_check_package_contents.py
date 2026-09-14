#!/usr/bin/env python3
from __future__ import annotations

import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import check_package_contents as checker


class PackageContentsTests(unittest.TestCase):
    def test_verify_uses_metadata_versions_in_archive_and_root_names(self) -> None:
        versions = {name: "9.8.7" for name in checker.EXPECTED}
        with tempfile.TemporaryDirectory() as directory:
            package_dir = Path(directory)
            for name, version in versions.items():
                archive = package_dir / f"{name}-{version}.crate"
                root = f"{name}-{version}/"
                with tarfile.open(archive, "w:gz") as crate:
                    for relative, content in {
                        "LICENSE": b"license",
                        "README.md": b"readme",
                        "Cargo.toml": (
                            b'license = "AGPL-3.0-or-later"\nreadme = "README.md"\n'
                        ),
                    }.items():
                        info = tarfile.TarInfo(root + relative)
                        info.size = len(content)
                        crate.addfile(info, io.BytesIO(content))
            # `verify` takes the denylist as an argument, so the test states
            # its own instead of reaching for the private file CI cannot have.
            # A term no fixture contains keeps `verify` returning [] for the
            # right reason rather than because the scan was skipped.
            terms = ("a-term-no-fixture-contains",)
            self.assertEqual(checker.verify(package_dir, versions, terms), [])

    @patch("check_package_contents.subprocess.check_output")
    def test_workspace_versions_rejects_missing_expected_package(self, output) -> None:
        output.return_value = json.dumps({"packages": []})
        with self.assertRaisesRegex(ValueError, "missing workspace packages"):
            checker.workspace_versions()


if __name__ == "__main__":
    unittest.main()
