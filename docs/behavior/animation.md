# Aircraft part animation

Status: **specified** for control surfaces, flaps and landing gear.

## Node transforms
A hierarchy node (`him.md`) can carry an extra local rotation and translation on top of its `Attaching`
transform. The runtime sets them with "chunk" calls naming the node:
- `setAngles(node, a, b, c)`: degrees. The local rotation is `Rz(a) · Ry(−b) · Rx(−c)`, about the node's
  own axes (verified from the engine's matrix builder). Here `Rz/Ry/Rx` are right-handed rotations, and the
  combined matrix uses the same Euler composition as aircraft orientation.
- `setLocate(node, xyz, ypr)`: a local translation `xyz` (metres) plus angles as above.
- `visible(node, bool)`.

A node's transform is `parent · Attaching · T(xyz) · R(angles)`. Values persist until set again.

## Inputs and ranges
- **rudder, elevator, aileron**: control surface positions in `[−1, 1]`.
- **flaps**: `[0, 1]`.
- **gear**: `[0, 1]`, where **1 = down** and 0 = up. The shipped meshes are modelled with the gear *retracted*
  (all angles 0), so a parked aircraft needs its gear animation applied.

Each aircraft defines, per input, a function from the input value to a set of node changes. The generic
default (used when an aircraft defines nothing else) is:
- rudder: `Rudder1_D0` pitch `−30·r`.
- elevator: `VatorL_D0` and `VatorR_D0` pitch `−30·e`.
- aileron: `AroneL_D0` and `AroneR_D0` pitch `−30·a`.
- flaps: `Flap01_D0` … `Flap04_D0` pitch `−50·f`.
- gear: none.

## Importer (OpenIL2)
The per-aircraft functions live in the user's installed runtime classes, as small methods that compute angles
from the input value. OpenIL2 does not ship them. At load time it evaluates these methods in a restricted
interpreter:
- **allowed**: float, double and int arithmetic and comparisons, branches, constants, local variables,
  `Math.sin/cos/abs/min/max`, the runtime's clamp-and-map helper `cvt(x, x0, x1, y0, y1)`
  (`y0 + (y1 − y0)·clamp((x − x0)/(x1 − x0), 0, 1)`), the shared scratch arrays used with `setLocate`, the
  chunk calls above, and calls to other such methods along the class chain.
- It **samples** each function at 41 points over its input range and stores the node states.
- The runtime then interpolates linearly between samples.
- Static fields: a class's static initializer runs once, on first use of one of its fields, in a lenient mode
  where unknown calls return zero values, and the fields it assigns are kept (gear angle tables are static
  `float[]` literals). Unassigned static fields and the fields of `this` start at zero/false and keep values
  the methods assign.
- `chunkFindCheck` reports the node as present.
- Any instruction outside this set (reading flight-model state, randomness, `World.cur()`, other objects'
  fields) aborts that method, and the default above is used for that input.

Coverage (2026-10-16, 404 aircraft × 5 channels): all but about 50 channels import; the rest read live
flight-model state.

The methods sampled are `moveRudder(F)`, `moveElevator(F)`, `moveAileron(F)`, `moveFlap(F)` (instance methods,
resolved along the class chain), and the gear statics `moveGear(HierMesh, F, F, F)` (left, right and tail/nose
gear, all driven by the same value) plus `moveGear(HierMesh, F)`.
