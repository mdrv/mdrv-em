# mdrv-em — semantic emoji picker

A bemoji replacement: GPUI layer-shell panel summoned on `Super+.`,
type-to-search over the full Unicode catalog, **Enter copies + hides**.

Search runs entirely on-device: an embedded `bge-small-en-v1.5` int8
bundle (semantic) over CLDR names/keywords (exact + substring fallback
with configurable locales, `id` by default). No API at runtime — the
only LLM touch is a one-time batch that writes usage descriptions
(`scripts/describe.py`, template-driven), which are embedded as data.

## Layout

- single bin crate `mdrv-em`; the gpui fork comes via git tag (Policy B)
- `scripts/gen_data.py` — emoji-test.txt + CLDR annotations →
  `src/emoji_data.rs` (committed) + `data/catalog.json`
- `scripts/describe.py` — one-time Gemini batch → `data/descriptions.json`
- `deploy/mdrv-em.service` — systemd --user unit (`run --hidden`)

## Verbs

| verb                 | meaning                                  |
| -------------------- | ---------------------------------------- |
| `mdrv-em` / `toggle` | summon/dismiss (spawns a daemon if none) |
| `show` / `hide`      | one-directional                          |
| `stop`               | stop the daemon                          |
| `run [--hidden]`     | start the daemon (unit form)             |

## Config — `~/.config/mdrv-em/config.toml`

```toml
[panel]
anchor = "bottom-left" # 9 anchors: edges or center combos
margin = 12
width = 640
height = 420

[font]
emoji = "Noto Color Emoji" # family name or absolute .ttf path (Twemoji SVGinOT is NOT supported)

[search]
fallback_locales = ["id"] # CLDR keyword locales beyond en
max_results = 60

[bundle]
path = "" # alternate engine bundle (model.onnx + vectors.bin)
```

State: `~/.local/state/mdrv-em/recents.json`.

## Status

v0.0.1: keyword engine + panel lifecycle. Semantic bundle + CI releases
pending — see `docs/` and the overview in `/x/m/v270/mdrv-em/`.
