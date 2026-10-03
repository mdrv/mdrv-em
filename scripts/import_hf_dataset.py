#!/usr/bin/env python3
"""Import HF badrex/LLM-generated-emoji-descriptions into data/descriptions.json.

Provenance: Llama-3-8B-generated descriptions for 5,034 emoji (CC license),
https://huggingface.co/datasets/badrex/LLM-generated-emoji-descriptions

Merge priority: existing entries in data/descriptions.json (our own
describe.py / subagent style, lowercase <=140 chars) win over dataset rows.
Dataset `unicode` cells ("U+1F947", "U+1F170 FE0F", "U+1F3F4 E0067 ...")
are normalized to our cp-key form ("1F947", "1F170-FE0F", "1F3F4-E0067-...").
"""

import json
import pathlib
import subprocess
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
PARQUET = pathlib.Path("/tmp/opencode/emoji-desc.parquet")
URL = (
    "https://huggingface.co/datasets/badrex/LLM-generated-emoji-descriptions"
    "/resolve/main/data/train-00000-of-00001.parquet"
)
OUT = ROOT / "data" / "descriptions.json"


def norm_cp(unicode_cell: str) -> str:
    parts = unicode_cell.replace("U+", "").split()
    return "-".join(p.upper() for p in parts)


def main() -> None:
    if not PARQUET.exists() or PARQUET.stat().st_size < 100_000:
        print("downloading parquet ...")
        req = urllib.request.Request(URL, headers={"User-Agent": "mdrv-em/0.1"})
        with urllib.request.urlopen(req, timeout=120) as r, PARQUET.open("wb") as f:
            f.write(r.read())

    import pandas as pd

    df = pd.read_parquet(PARQUET)
    catalog = json.loads((ROOT / "data" / "catalog.json").read_text())
    done: dict[str, str] = json.loads(OUT.read_text()) if OUT.exists() else {}

    ours = len(done)
    added = 0
    for _, row in df.iterrows():
        cp = norm_cp(row["unicode"])
        desc = str(row["LLM description"]).strip()
        if cp and desc and cp not in done:
            done[cp] = desc
            added += 1

    OUT.write_text(json.dumps(done, ensure_ascii=False, indent=1))
    cat_cps = {e["cp"] for e in catalog}
    covered = sum(1 for cp in cat_cps if cp in done)
    missing = sorted(cat_cps - set(done))
    print(f"kept {ours} existing, imported {added} from dataset")
    print(f"catalog coverage: {covered}/{len(cat_cps)}")
    if missing:
        miss_path = ROOT / "data" / "descriptions-missing.json"
        miss_path.write_text(json.dumps(missing, ensure_ascii=False, indent=1))
        print(f"{len(missing)} missing -> {miss_path.relative_to(ROOT)}")
    # pyarrow/pandas are only needed here; surface import errors late on purpose
    _ = subprocess  # keep import list minimal


if __name__ == "__main__":
    sys.exit(main())
