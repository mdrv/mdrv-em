#!/usr/bin/env python3
"""One-time batch: write a usage description per emoji.

Input:  data/catalog.json (emitted by gen_data.py)
Output: data/descriptions.json  {cp: description} — rerun gen_data.py
        afterwards so descriptions land in src/emoji_data.rs.

Providers (-P/--provider):
  gemini      Google AI Studio native API (default; GEMINI_API_KEY)
  groq        GroqCloud free tier (GROQ_API_KEY; ~30 RPM / 1K req/day)
  nvidia      NVIDIA NIM free tier (NVIDIA_API_KEY; ~40 RPM)
  openrouter  OpenRouter free models (OPENROUTER_API_KEY)
  custom      any OpenAI-compatible endpoint: --base-url + CUSTOM_API_KEY
              (or OPENAI_API_KEY); e.g. AMD Radeon Token Factory

Everything except gemini speaks the OpenAI chat format. Resumable: cps
already present in the output file are skipped; the file is rewritten
every 25 results. Free tiers cap RPM/RPD — pass --rpm to pace (groq 28,
nvidia 38) and simply rerun on the next day when the daily cap trips;
the retry loop rides out short 429s.

Usage:
  python3 scripts/describe.py                          # gemini
  python3 scripts/describe.py -P groq --rpm 28
  python3 scripts/describe.py -P nvidia --rpm 38
  python3 scripts/describe.py -P custom --base-url https://… --model …
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import threading
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "data" / "catalog.json"
OUT = ROOT / "data" / "descriptions.json"
TEMPLATE = ROOT / "scripts" / "templates" / "describe.md"
GEMINI_API = (
    "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
)

PROVIDERS: dict[str, dict] = {
    "gemini": {"env": "GEMINI_API_KEY", "model": "gemini-2.5-flash-lite"},
    "groq": {
        "env": "GROQ_API_KEY",
        "base": "https://api.groq.com/openai/v1",
        "model": "openai/gpt-oss-20b",
        "rpm": 28,
    },
    "nvidia": {
        "env": "NVIDIA_API_KEY",
        "base": "https://integrate.api.nvidia.com/v1",
        "model": "openai/gpt-oss-20b",
        "rpm": 38,
    },
    "openrouter": {
        "env": "OPENROUTER_API_KEY",
        "base": "https://openrouter.ai/api/v1",
        "model": "openai/gpt-oss-20b:free",
    },
    "amd": {
        "env": "RADEON_API_KEY",
        "base": "https://developer.amd.com.cn/radeon/api/v1",
        "model": "GLM-5.3-Flash",
    },
}


class HttpError(Exception):
    def __init__(self, code: int):
        super().__init__(f"http {code}")
        self.code = code


def post_json(url: str, body: bytes, headers: dict) -> dict:
    req = urllib.request.Request(
        url,
        data=body,
        headers={
            "Content-Type": "application/json",
            # Groq's WAF 403s the default Python-urllib User-Agent
            "User-Agent": "mdrv-em-describe/0.1",
            **headers,
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=90) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        raise HttpError(e.code) from e


def call_gemini(key: str, model: str, prompt: str) -> str:
    data = post_json(
        GEMINI_API.format(model=model),
        json.dumps(
            {
                "contents": [{"parts": [{"text": prompt}]}],
                "generationConfig": {"temperature": 0.3, "maxOutputTokens": 512},
            }
        ).encode(),
        {"x-goog-api-key": key},
    )
    return data["candidates"][0]["content"]["parts"][0]["text"]


def call_oai(key: str, base: str, model: str, prompt: str) -> str:
    body: dict = {
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.3,
        # reasoning models spend completion tokens on hidden reasoning first
        "max_tokens": 1024,
    }
    if "gpt-oss" in model:
        body["reasoning_effort"] = "low"
    data = post_json(
        f"{base.rstrip('/')}/chat/completions",
        json.dumps(body).encode(),
        {"Authorization": f"Bearer {key}"},
    )
    msg = data["choices"][0]["message"]
    return msg.get("content") or ""


_pace_lock = threading.Lock()
_pace_next = 0.0


def paced(rpm: int, call: Callable[[str], str]) -> Callable[[str], str]:
    """Serialize call starts to at most `rpm` per minute (0 = unpaced)."""
    if rpm <= 0:
        return call

    def wrapped(prompt: str) -> str:
        global _pace_next
        with _pace_lock:
            now = time.monotonic()
            wait = max(0.0, _pace_next - now)
            _pace_next = max(now, _pace_next) + 60.0 / rpm
        if wait > 0:
            time.sleep(wait)
        return call(prompt)

    return wrapped


def describe_one(
    entry: dict, template: str, call: Callable[[str], str]
) -> tuple[str, str | None, str | None]:
    prompt = template.format(
        ch=entry["ch"],
        name=entry["name"],
        group=entry["group"],
        keywords=", ".join(entry.get("kws_en", [])[:8]),
    )
    for attempt in range(6):
        try:
            text = (call(prompt) or "").strip()
            if not text:
                raise RuntimeError("empty content")
            return entry["cp"], text.splitlines()[0][:140], None
        except HttpError as e:
            if e.code in (429, 500, 502, 503):
                time.sleep(2**attempt)
                continue
            return entry["cp"], None, str(e)
        except Exception as e:  # network hiccups
            if attempt == 5:
                return entry["cp"], None, str(e)
            time.sleep(2**attempt)
    return entry["cp"], None, "retries exhausted"


def make_call(args) -> tuple[Callable[[str], str], str]:
    """Wire up the provider; returns (call, label)."""
    if args.provider == "gemini":
        key = os.environ.get("GEMINI_API_KEY")
        if not key:
            sys.exit("GEMINI_API_KEY not set")
        model = args.model or PROVIDERS["gemini"]["model"]
        return (lambda p: call_gemini(key, model, p)), f"gemini/{model}"
    if args.provider == "custom":
        if not args.base_url:
            sys.exit("--base-url required with -P custom")
        if not args.model:
            sys.exit("--model required with -P custom")
        key = os.environ.get("CUSTOM_API_KEY") or os.environ.get("OPENAI_API_KEY")
        if not key:
            sys.exit("CUSTOM_API_KEY (or OPENAI_API_KEY) not set")
        return (lambda p: call_oai(key, args.base_url, args.model, p)), (
            f"custom/{args.model}"
        )
    conf = PROVIDERS[args.provider]
    key = os.environ.get(conf["env"])
    if not key:
        sys.exit(f"{conf['env']} not set")
    model = args.model or conf["model"]
    rpm = args.rpm or conf.get("rpm", 0)
    return paced(rpm, lambda p: call_oai(key, conf["base"], model, p)), (
        f"{args.provider}/{model}"
    )


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "-P", "--provider", default="gemini", choices=list(PROVIDERS) + ["custom"]
    )
    ap.add_argument("--model", default=None, help="override the provider default")
    ap.add_argument(
        "--base-url", default=None, help="custom OpenAI-compatible endpoint"
    )
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument(
        "--rpm", type=int, default=0, help="request pace; 0 = provider default/off"
    )
    ap.add_argument("--out", default=None, help="override output json path")
    args = ap.parse_args()

    call, label = make_call(args)
    out = Path(args.out) if args.out else OUT
    catalog = json.loads(CATALOG.read_text())
    done: dict[str, str] = json.loads(out.read_text()) if out.exists() else {}
    template = TEMPLATE.read_text()

    todo = [e for e in catalog if e["cp"] not in done]
    print(f"{len(done)} already described, {len(todo)} to go ({label})")
    if not todo:
        return

    failures = 0
    consec = 0
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = [pool.submit(describe_one, e, template, call) for e in todo]
        for i, fut in enumerate(as_completed(futures), 1):
            cp, desc, err = fut.result()
            if desc:
                done[cp] = desc
                consec = 0
            else:
                failures += 1
                consec += 1
                print(f"  FAIL {cp}: {err}", file=sys.stderr)
                if consec >= 12:
                    out.write_text(json.dumps(done, ensure_ascii=False, indent=1))
                    sys.exit(
                        f"aborting: {consec} consecutive failures — provider is down;"
                        " progress saved, rerun later"
                    )
            if i % 25 == 0:
                out.write_text(json.dumps(done, ensure_ascii=False, indent=1))
                print(f"  {i}/{len(todo)}")
    out.write_text(json.dumps(done, ensure_ascii=False, indent=1))
    print(f"done: {len(done)} described, {failures} failed")


if __name__ == "__main__":
    main()
