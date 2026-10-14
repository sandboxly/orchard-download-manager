"""Build the website into _site/: a copy of site/ without the build files.

    python3 site/build.py    then serve _site/ (python3 -m http.server -d _site)
"""

from __future__ import annotations

import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
OUT = ROOT / "_site"


def build() -> None:
    shutil.rmtree(OUT, ignore_errors=True)
    shutil.copytree(SITE, OUT, ignore=shutil.ignore_patterns("build.py", "__pycache__", ".DS_Store"))
    print(f"Built {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    build()
