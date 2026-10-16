# Multiplayer (OpenIL2's own protocol)

The original's network game (and its `.ntrk` tracks) use the runtime's own replication protocol,
which OpenIL2 doesn't speak yet. OpenIL2 has its own: a host and clients over UDP, each machine
flying its own aircraft.

```sh
openil2 --host 27015 --name Anna --aircraft air.BF_109G6 --map maps/Slovakia/load.ini --at 214500,86500 --agl 1000
openil2 --join 192.168.1.10:27015 --name Ben --aircraft air.LAGG_3SERIES4
```

The host's options (map or mission, position, time; not its aircraft) are sent to each joining
pilot. That pilot starts 1.5 km from the host's start in a direction set by its number. Missions
fly without their AI aircraft and ground columns: each machine would fly its own copy.

Messages (first byte the type, then fields as in the track format, `docs/formats/track.md`):

| type | contents | direction |
|---|---|---|
| 1 hello | name, aircraft class, army | client → host, repeated until answered |
| 2 welcome | pilot number, flight options | host → client |
| 3 pilot | number, name, class, army | host → all (the host's own aircraft included) |
| 4 state | aircraft state, 20 times a second | each → host → others |
| 5 damage | target pilot, chunk, levels | shooter → host → all |
| 6 rounds | shooter, rounds fired | each → host → others |
| 7 bye | pilot number | leaving pilot → host → others |
| 8 chat | pilot number, text | each → host → others |

- Remote aircraft are shown 0.1 s behind real time, interpolated between the states around then.
- The shooter decides hits: rounds that hit a remote aircraft's chunks work out the damage
  levels locally (generic chunk damage, `damage.md`) and send them. The owner applies them to its
  aircraft, others to their copies.
- Other pilots' rounds are shown but do no damage where they arrive.
