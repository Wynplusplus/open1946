# OpenIL2

A clean-room reimplementation of the IL-2 Sturmovik 1946 engine in Rust, in the spirit of OpenMW.
It needs a legally owned copy of the game (e.g. GOG); point `IL2_ROOT` at the install directory.

No game files, decompiled code or key material are part of this repository. Format knowledge lives in
`docs/formats/` as prose specifications, and the Rust code is written from those specs.

## Status: milestones 1–6 done

| Area | State |
|------|-------|
| SFS archives (`il2-sfs`) | All 76 archives, 266,627 files, 0 decode failures |
| Mount order and lookup (`il2-vfs`) | Matches the game: `.rc` mount list, later mounts win, protected dirs |
| Section files, binary section files | Text and binary encodings |
| Meshes `.msh`, hierarchies `.him`/`.sim`, materials `.mat` | 90,525 / 1,555 / 43,464 shipped files parse, 0 failures |
| Images | IMF types 0–2, TGA including colour-mapped variants |
| Maps | Height map, land-type texturing, water, forests, roads and railways, bridges, sun and sky by time and date |
| Map objects | Houses, tree lines, runway and taxiway plates from `*.static` + `static.ini`, instanced; all 102 shipped maps load |
| Mission objects | Stationary vehicles, guns, ships, parked aircraft, balloons, buildings; all 1,284 shipped missions resolve |
| Flight (`il2-fm`, `il2-data`) | All 404 aircraft fly: piston engines with boost, jets, rockets, gliders; median speed error 0 % low / −1.6 % high against the `.fmd` figures (`docs/behavior/validation.md`); engine start-up, temperatures, radiators; trim, flaps, dive brakes; the runtime's gear model; joystick |
| Game (`openil2`) | Fly any aircraft class over any map or from a mission's start (runway takeoff spots) |
| Cockpits | All 241 player cockpits, with every instrument driven by the cockpit's own code from the install; head look |
| Aircraft damage | Damage levels, parts torn off as tumbling wreckage, flight-model losses, G limits, flutter and ground strikes |
| Weapons | Loadouts and gun, bomb and rocket data from the install; ballistics, tracers, house destruction |
| Effects and sound | Particle effects from `.eff` files for impacts and explosions; engine, gun and explosion sounds from the game's presets (ALSA) |
| AI and the world | AI flights fly missions (takeoff, waypoints, formation, air combat, level bombing, landing), gunners, anti-aircraft guns, moving ground and naval columns |
| Menus | Single missions with localized briefings, Pilot Career (static campaigns), dynamic campaigns from the DGen data, Quick Mission Builder, Play Track |
| More | Mission objectives and results, track recording and playback (own format), multiplayer (own protocol), DeviceLink |

Where OpenIL2 uses its own designs instead of the original's code, and what is not reproduced yet
(original network protocol and `.ntrk` tracks, the runtime's AI maneuvers, the detailed damage
model, …), is listed in `docs/roadmap.md`.

## Usage

```sh
export IL2_ROOT="$HOME/Games/Heroic/IL-2 Sturmovik 1946"
cargo build --release

# Archive tools
target/release/il2 validate                 # decode every block of every archive
target/release/il2 stat
target/release/il2 cat com/maddox/il2/objects/air.ini
target/release/il2 names --out names.txt    # recover archive path names (~75%)
target/release/il2 parse-all --names names.txt

# Menus: single missions with briefings, Quick Mission Builder (no options = menus)
target/release/openil2

# Fly (arrows: stick, Z/X: rudder, PageUp/PageDown or 1-9/0/W: throttle, F/V: flaps, G: gear, B: brakes,
#      I: start/stop engines, R: radiator (auto / 33 / 67 / 100 %), W: emergency boost, ,/.: elevator trim, [/]: aileron trim, ;/': rudder trim, N: dive brake, L: tail wheel lock, O: prop pitch auto/manual, Q/E: prop lever,
#      Space: guns, Enter: cannons, U: rockets, K: bombs,
#      F1: cockpit (mouse drag looks around), F2: free orbit view with mouse drag and wheel, F3: chase view)
target/release/openil2 --aircraft air.BF_109G6 --map maps/Slovakia/load.ini --at 214500,86500 --agl 300
target/release/openil2 --mission Missions/Single/SK/Bf-109F-4/Bf-109F-4_1.mis
target/release/openil2 --test                                   # headless flight-model checks
target/release/openil2 --at 214500,86500 --time 19.5              # dusk; --time overrides the mission's
target/release/openil2 --at 214500,86500 --agl 150 --screenshot out.png --orbit 170,12,40
target/release/openil2 --screenshot out.png --cockpit=-30,-10        # from the cockpit, head turned
target/release/openil2 --screenshot out.png --damage WingLMid_D0 --seconds 0.5 --orbit 150,30,16
target/release/openil2 --mission Missions/Single/SK/Bf-109F-4/Bf-109F-4_1.mis --record flight.otrk
target/release/openil2 --play flight.otrk
target/release/openil2 --host 27015 --aircraft air.BF_109G6 --map maps/Slovakia/load.ini --at 214500,86500 --agl 1000
target/release/openil2 --join 192.168.1.10:27015 --aircraft air.LAGG_3SERIES4
target/release/openil2 --devicelink 10000          # or [DeviceLink] port=10000 in conf.ini

# Joystick (Linux /dev/input/js*): picked up automatically; default axes X/Y = stick, 2 = throttle,
# 3 = twist rudder, button 2 = wheel brakes. Remap with e.g. --joy-map aileron=0,elevator=1,rudder=5,throttle=-2

# Viewer (drag to orbit, wheel to zoom)
target/release/il2-viewer "3DO/Plane/Bf-109G-6Early/hier.him"
target/release/il2-viewer "3do/plane/Il-2-1941Early(Multi1)/hier.him" --season winter
target/release/il2-viewer maps/Slovakia/load.ini
target/release/il2-viewer maps/Slovakia/load.ini --screenshot out.png --zoom 0.1 --target 150000,100000,1500
```

## Tests

```sh
cargo test --workspace
IL2_ROOT=... cargo test --workspace -- --ignored   # against a real install
```
