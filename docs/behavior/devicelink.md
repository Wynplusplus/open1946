# DeviceLink

The UDP interface for external tools, as the install's `DeviceLink.txt` specifies it. OpenIL2
serves it when the install's `conf.ini` has a `[DeviceLink]` section (`port`, optional `host`, and
`IPS`, the allowed client addresses) or with `--devicelink PORT`.

- A query is `R` followed by keys `/<number>`, each optionally followed by values `\<value>`. A
  `\` before `\` or `/` escapes it. Even keys get, odd keys set. Answers come back in packets
  starting with `A`, one `/<key>\<value>…` per get key, packed into packets of at most 2048
  bytes. Numbers have two decimals (the runtime's format).
- Engine readings take the engine index as their value and answer `/<key>\<index>\<value>`.
- Keys `4` and `6` answer whether a key has a get or set method: `/4\<key>\<0|1>`.

OpenIL2 answers 2 (version), 4, 6, 20 (time of day), 22 (plane), 24 (cockpits), 26 (cockpit
index, −1 outside), 28 (engines), 30–60 (indicated speed in km/h, climb rate, slip, turn, turn
rate, altitude, compass heading, roll, pitch, fuel, load factor, shake 0, gear positions), 62–74
(engine: magneto 3, rpm, manifold pressure in bar, oil in/out, water and cylinder temperatures),
80–98 (control positions), 104 (WEP), 164 (gear), 172 (air brake), 174 (tail lock) and 180–188
(weapons firing).
It accepts 81–99 (power, flaps, aileron, elevator, rudder, brakes, propeller pitch, trims; −1…1),
103 (engine on/off), 105 (WEP), 165 (gear), 173 (air brake), 175 (tail lock) and 181–189
(weapons, `1` start, `0` stop). Stick axes set this way hold until the keyboard moves them.
Camera, padlock, radio and the dedicated-server keys (1001+) are not answered.
