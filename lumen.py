"""lumen: control the Xiaomi/Yeelight lights over the local network (miIO, UDP 54321).

  lumen                       status of all lights
  lumen on|off|toggle [LIGHT]
  lumen bright 0-100 [LIGHT]
  lumen color NAME|#RRGGBB [LIGHT]     (bulb, strip)
  lumen temp 1700-6500 [LIGHT]          (desk lamp: 2700-6500)
  lumen night|read|focus [LIGHT]        presets

LIGHT is desk, bulb, strip, or a comma list; default is all of them.
Tokens live in devices.json next to this file (made from the cloud once).
"""
import json
import os
import sys
import warnings
from concurrent.futures import ThreadPoolExecutor

warnings.filterwarnings("ignore")
from miio import Device  # noqa: E402

HERE = os.path.dirname(os.path.realpath(__file__))
FADE = 300  # ms

COLORS = {
    "red": 0xFF0000, "orange": 0xFF7000, "yellow": 0xFFD000, "green": 0x00FF40,
    "cyan": 0x00FFFF, "blue": 0x0040FF, "purple": 0x8000FF, "pink": 0xFF40A0,
    "white": 0xFFFFFF,
}
PRESETS = {  # (brightness, colour temperature)
    "night": (5, 2700),
    "read": (80, 4000),
    "focus": (100, 5500),
}
COLOR_CAPABLE = {"yeelink.light.color1", "yeelink.light.strip1"}


def load():
    with open(os.path.join(HERE, "devices.json")) as f:
        return {d["alias"]: d for d in json.load(f)}


def pick(lights, arg):
    if not arg or arg == "all":
        return list(lights.values())
    names = arg.split(",")
    bad = [n for n in names if n not in lights]
    if bad:
        sys.exit(f"unknown light: {', '.join(bad)} (have: {', '.join(lights)})")
    return [lights[n] for n in names]


def dev(d):
    return Device(d["ip"], d["token"], timeout=2)


def status(d):
    keys = ["power", "bright", "ct", "rgb", "color_mode"]
    p = dict(zip(keys, dev(d).send("get_prop", keys)))
    if p["power"] != "on":
        return f"{d['alias']:6} off"
    look = f"#{int(p['rgb']):06x}" if p["color_mode"] == "1" else f"{p['ct']} K"
    return f"{d['alias']:6} on   {p['bright']:>3}%  {look}"


def ensure_on(D):
    # Yeelights ignore colour/brightness while off, so switch on first.
    D.send("set_power", ["on", "smooth", FADE])


def run(d, cmd, val):
    D = dev(d)
    if cmd in ("on", "off"):
        D.send("set_power", [cmd, "smooth", FADE])
    elif cmd == "toggle":
        D.send("toggle", [])
    elif cmd == "bright":
        ensure_on(D)
        D.send("set_bright", [max(1, min(100, int(val))), "smooth", FADE])
    elif cmd == "temp":
        ensure_on(D)
        lo = 2700 if d["model"] == "yeelink.light.lamp4" else 1700
        D.send("set_ct_abx", [max(lo, min(6500, int(val))), "smooth", FADE])
    elif cmd == "color":
        if d["model"] not in COLOR_CAPABLE:
            return f"{d['alias']:6} has no colour, skipped"
        rgb = COLORS.get(val.lower()) if val.lower() in COLORS else int(val.lstrip("#"), 16)
        ensure_on(D)
        D.send("set_rgb", [max(1, rgb), "smooth", FADE])
    elif cmd in PRESETS:
        b, ct = PRESETS[cmd]
        ensure_on(D)
        lo = 2700 if d["model"] == "yeelink.light.lamp4" else 1700
        D.send("set_ct_abx", [max(lo, ct), "smooth", FADE])
        D.send("set_bright", [b, "smooth", FADE])
    return status(d)


def main(argv):
    lights = load()
    if not argv or argv[0] in ("status", "list"):
        targets, work = pick(lights, argv[1] if len(argv) > 1 else None), status
    elif argv[0] in ("-h", "--help", "help"):
        print(__doc__.strip())
        return
    else:
        cmd = argv[0]
        takes_val = cmd in ("bright", "temp", "color")
        if cmd not in ("on", "off", "toggle", *PRESETS) and not takes_val:
            sys.exit(f"unknown command: {cmd}\n\n{__doc__.strip()}")
        if takes_val and len(argv) < 2:
            sys.exit(f"{cmd} needs a value")
        val = argv[1] if takes_val else None
        targets = pick(lights, argv[2] if takes_val and len(argv) > 2 else
                       (argv[1] if not takes_val and len(argv) > 1 else None))
        if cmd == "color" and val.lower() not in COLORS:
            try:
                int(val.lstrip("#"), 16)
            except ValueError:
                sys.exit(f"colour must be #RRGGBB or one of: {', '.join(COLORS)}")
        work = lambda d: run(d, cmd, val)  # noqa: E731

    def safe(d):
        try:
            return work(d)
        except Exception as e:
            return f"{d['alias']:6} not reachable ({e.__class__.__name__})"

    with ThreadPoolExecutor(len(targets)) as ex:
        for line in ex.map(safe, targets):
            print(line)


if __name__ == "__main__":
    main(sys.argv[1:])
