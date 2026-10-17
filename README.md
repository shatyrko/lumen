# Lumen

Your Xiaomi/Yeelight lights from the tray, over the local network only: no Mi Home app, no
cloud, answers in 10-30 ms.

- **Panel** (click the tray bulb): every light with its switch and brightness; open a card for
  - **Colour** (bulb, strip): colour wheel (drag), 16 colour presets, `#RRGGBB`
  - **White**: colour temperature (desk lamp 2600-5000 K, bulb and strip 1700-6500 K) and the
    presets Reading, Computer, Focus, Relax, Night
  - **Scenes** (bulb, strip): Candle, Sunrise, Sunset, Romance, Ocean, Lava, Rainbow, Party,
    Police, Breathe
- **Tray**: middle click switches all lights on/off, scroll changes the brightness of the lights
  that are on, right click has a menu (each light, All on/off, white presets for all).
- **Moved lights**: when a light stops answering, Lumen asks the network for its device id and
  saves the new address.

## Command line

```
lumen                       start the tray app
lumen panel                 open/close the panel (for a keyboard shortcut)
lumen status
lumen on|off|toggle [LIGHT]          LIGHT = desk, bulb, strip, a comma list; default all
lumen bright 40|+10|-10 [LIGHT]
lumen temp 3000 [LIGHT]
lumen color blue|#ff8800 [LIGHT]
lumen preset reading|computer|focus|relax|night [LIGHT]
lumen scene candle|...|stop [LIGHT]
lumen search                look for the lights again
lumen autostart on|off
```

## Setup (done once)

The lights speak Xiaomi's miIO protocol (UDP 54321), encrypted with a per-light token. The
tokens come from the Xiaomi cloud once, with
[Xiaomi-cloud-tokens-extractor](https://github.com/PiotrMachowski/Xiaomi-cloud-tokens-extractor)
(kept in `vendor/`, not committed):

```
python3 -m venv .venv && .venv/bin/pip install -r vendor/requirements.txt
umask 077; .venv/bin/python -I vendor/token_extractor.py -o cloud.json
lumen import cloud.json     # writes ~/.config/lumen/lights.json, mode 600
```

A token changes only when a light is reset or removed from Mi Home and added again; then run
the two last commands again. Xiaomi has switched local control off on some models with firmware
updates, so it is safer not to update the lights' firmware.

## Build

```
cargo build --release && install -Dm755 target/release/lumen ~/.local/bin/lumen
lumen install-launcher
```

Rust, GTK4 + libadwaita, gtk4-layer-shell (the panel), ksni (the tray icon), like Penumbra.
`cargo test` checks the protocol byte for byte against python-miio.
