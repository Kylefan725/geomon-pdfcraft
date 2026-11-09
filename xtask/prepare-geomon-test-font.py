"""Add the GeoMon PDF processor's verified Hong Kong font to the shared CI font checkout."""

import hashlib
import os
from pathlib import Path
import tomllib
from urllib.request import urlopen


def fetch_verified(path: Path, url: str, sha256: str) -> None:
    if path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == sha256:
        return
    with urlopen(url, timeout=120) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != sha256:
        raise SystemExit(f"Checksum mismatch for {path.name}")
    path.write_bytes(data)


def main() -> None:
    repo = Path(__file__).resolve().parents[1]
    with (repo / "ATTRIBUTION.toml").open("rb") as attribution:
        font = next(entry for entry in tomllib.load(attribution)["fetched"] if entry["file"] == "NotoSansCJKhk-Regular.otf")
    checkout = Path(os.environ["CRAFT_FONTS_DIR"])
    manifest = checkout / "fonts/manifest.txt"
    rows = manifest.read_text(encoding="utf-8").splitlines()
    directory = checkout / "fonts/geomon"
    directory.mkdir(exist_ok=True)
    fetch_verified(directory / font["file"], font["url"], font["sha256"])
    fetch_verified(directory / "OFL.txt", font["licence_url"], font["licence_sha256"])
    row = " | ".join((
        "Noto Sans CJK HK", "Regular", f"fonts/geomon/{font['file']}", "Hant,Hans,Jpan,Latn",
        font["licence"], "fonts/geomon/OFL.txt", font["sha256"], font["source"],
    ))
    if row not in rows:
        with manifest.open("a", encoding="utf-8") as destination:
            destination.write(f"\n{row}\n")


if __name__ == "__main__":
    main()
