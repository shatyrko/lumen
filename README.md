# lumen

Controls the Xiaomi/Yeelight lights in my room over the local network (miIO, UDP 54321): no Mi Home app, no cloud.

    lumen                      # status
    lumen off desk
    lumen color blue bulb,strip
    lumen night                # all lights, 5 %, 2700 K

`devices.json` (ignored, mode 600) holds each light's IP and token. Tokens come from the Xiaomi cloud once, with
[Xiaomi-cloud-tokens-extractor](https://github.com/PiotrMachowski/Xiaomi-cloud-tokens-extractor) (`vendor/`, ignored):

    umask 077; .venv/bin/python -I vendor/token_extractor.py -o cloud.json

A token changes only if a light is reset or re-paired in Mi Home. If a light's IP changes, fix it in `devices.json`
(or give it a fixed DHCP lease in the router).
