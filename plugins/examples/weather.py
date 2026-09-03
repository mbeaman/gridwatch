#!/usr/bin/env python3
"""A gridwatch plugin in one file, with no dependencies.

It is a *source* (it publishes a metric) and a *component* (it draws a
tile), which is the most a plugin can be. Everything it needs to know is
in `docs/PLUGINS.md`; everything it is allowed to say is in
`schema/exec.schema.json`.

There is no network here on purpose: CI runs this file, and a test that
asks the internet for the weather is a test that fails when it rains on
someone else's server. It reads a temperature from a file if there is one
and otherwise makes a plausible one up, which is exactly what the demo
sources in the host do.

    [[plugins]]
    id = "weather"
    argv = ["python3", "plugins/examples/weather.py"]

Then place it like any other tile:

    place = [{ id = "outside", kind = "weather.weather", at = [0, 0], size = [2, 1] }]
"""

import json
import math
import os
import sys
import time

SOURCE_FILE = os.environ.get("WEATHER_FILE", "/tmp/gridwatch-weather")


def say(message):
    """One JSON object per line, flushed: the host reads line by line."""
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


MANIFEST = {
    "kind": "weather",
    "name": "outside",
    "summary": "the temperature outside, for contrast with the one inside",
    "contract": 1,
    "footprints": [{"w": 1, "h": 1}, {"w": 2, "h": 1}],
    "default_footprint": {"w": 2, "h": 1},
    # Poorest first, and the first one has to fit an 8x3 tile — the host
    # refuses a manifest whose smallest tier does not, because there would
    # be sizes at which the tile could draw nothing.
    "tiers": [
        {"name": "badge", "min": {"w": 8, "h": 3}, "adds": ["the temperature"]},
        {
            "name": "full",
            "min": {"w": 24, "h": 6},
            "adds": ["the trend", "where the reading came from"],
        },
    ],
    "sources": ["weather"],
    "produces": [
        {"key": "weather.temp_c", "unit": "celsius", "help": "outside temperature"}
    ],
    "keys": [{"key": "r", "does": "read it again"}],
}


def temperature():
    """Whatever the file says, or a slow sine so the tile has something to
    draw. The second half is honest about being invented: the tile says
    which one it is showing."""
    try:
        with open(SOURCE_FILE) as f:
            return float(f.read().strip()), True
    except (OSError, ValueError):
        # A day-long cycle around 14 °C, so a person watching for a minute
        # sees a number that moves a little.
        return 14.0 + 6.0 * math.sin(time.time() / 13750.0), False


def view(instance, tier, temp, real):
    """A view *tree*, not pixels: no colours, no cell positions. The host
    draws it with the same renderer it uses for its own tiles, which is why
    a plugin cannot break the theme."""
    reading = f"{temp:.1f}°C"
    if tier == 0:
        return {"text": [[{"role": "text", "text": reading}]]}
    where = SOURCE_FILE if real else "no reading — this number is invented"
    return {
        "stack": {
            "dir": "v",
            "children": [
                [{"len": 1}, {"text": [[{"role": "accent_primary", "text": reading}]]}],
                [
                    {"fill": 1},
                    {"text": [[{"role": "text_muted", "text": where}]]},
                ],
            ],
        }
    }


def main():
    # The host speaks first. Reading it is how a plugin learns the contract
    # number and what this machine can do.
    first = sys.stdin.readline()
    if not first:
        return
    hello = json.loads(first)
    if hello.get("contract") != 1:
        say({"kind": "status", "state": "unavailable",
             "reason": f"this plugin speaks contract 1, the host speaks {hello.get('contract')}"})
        return

    say({"kind": "manifest", "manifest": MANIFEST})
    temp, real = temperature()
    say({"kind": "status", "state": "ok" if real else "degraded",
         "reason": None if real else f"no {SOURCE_FILE}; showing an invented number",
         "hint": None if real else f"write a number into {SOURCE_FILE}"})
    say({"kind": "sample", "key": "weather.temp_c", "value": round(temp, 2)})

    # Then answer what the host asks. A plugin that has nothing to say
    # says nothing: this loop blocks on stdin and costs nothing while it
    # waits.
    for line in sys.stdin:
        try:
            ask = json.loads(line)
        except json.JSONDecodeError:
            continue
        kind = ask.get("kind")
        if kind == "render":
            temp, real = temperature()
            say({
                "kind": "view",
                "instance": ask["instance"],
                "tree": view(ask["instance"], ask.get("tier", 0), temp, real),
            })
            say({"kind": "sample", "key": "weather.temp_c", "value": round(temp, 2)})
        elif kind == "key" and ask.get("key") == "r":
            temp, real = temperature()
            say({"kind": "sample", "key": "weather.temp_c", "value": round(temp, 2)})
            say({"kind": "command",
                 "command": {"toast": {"severity": "info", "text": f"outside: {temp:.1f}°C"}}})


if __name__ == "__main__":
    main()
