# Tracks (`.otrk`, OpenIL2's own format)

The original's tracks are `records/*.ntrk`, encrypted recordings of the game's network
replication stream, and the older `*.TRK` key recordings. OpenIL2 can't play those yet (that needs
the network protocol). It records and plays its own state tracks instead.

All numbers are little-endian. Strings are a `u16` byte length, then UTF-8.

- `"OIL2TRK1"` (8 bytes).
- `u16` option count, then the options as strings: the command line that set the flight up
  (mission, aircraft, map, position…, without `--record`, `--play`, `--result`, `--root`,
  `--quit-after`). Playback sets up the same flight from them.
- Frames, every 0.05 s of flight time:
  - `u8` 1, `f64` time;
  - `u16` aircraft count, each: `u32` id (0 the player, `1 + n` the mission's AI aircraft `n`),
    position `3 × f64`, orientation quaternion `4 × f32` (x y z w), velocity `3 × f32`, controls
    `7 × f32` (aileron, elevator, rudder, flaps, gear, throttle, air brake), `u8` destroyed;
  - `u32` ground unit count, each: position `3 × f32`, heading `f32`, `u8` alive (all columns'
    units in mission order);
  - `u16` damage events: `u32` aircraft id, chunk string, `u8` damage levels;
  - `u32` destroyed map objects: `u32` object index;
  - `u16` explosions: position `3 × f64`, `f32` kg TNT;
  - `u32` rounds fired since the last frame: position `3 × f64`, velocity `3 × f32`, gun name
    string, `u8` bullet type, `u8` tracer, `f32` life left, `f32` fuse (−1: none).

Playback interpolates aircraft between frames (linear position, spherical orientation) and
replays the events as they pass: damage levels on the same chunks, map objects destroyed,
explosions and rounds (for show; they do no damage). Flights started from the menus are recorded
to `$XDG_DATA_HOME/openil2/tracks/<unix time>.otrk`, keeping the last 20, and the Play Track
screen lists them. `openil2 --record FILE` and `openil2 --play FILE` work directly.
