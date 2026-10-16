# Cockpit view

## Which cockpit an aircraft uses

An aircraft class names its cockpit in its static initializer as the property `cockpitClass`
(one class, or an array of classes for multi-crew aircraft whose first entry is the pilot's
seat). The cockpit class's constructor passes its hierarchy file, a path under `3do/cockpit/`
ending in `.him`, to the cockpit base class. OpenIL2 finds the first such string constant in the
cockpit class.

Of the 404 classes in `air.ini`, 241 have a cockpit. The rest are AI-only types and the "ace"
variants (`BF_109G6GRAF`, `LA_7KOJEDUB`, …), which the original doesn't let the player fly from
the cockpit either. For these OpenIL2 falls back to the chase view.

## Placement

The cockpit actor sits on the aircraft with a hook that copies only the orientation. Its origin
is the aircraft's origin and its axes are the body axes, so the cockpit mesh is in body
coordinates. The pilot's eye is the cockpit mesh's `CAMERA` hook (`CAMERAAIM` is the eye when
aiming through the gunsight). The world camera takes the same point: the aircraft position plus
the rotated `CAMERA` offset. Head movement turns the view around that point.

The original draws the cockpit in its own pass with its own camera and near clip plane. OpenIL2
does the same: the world is drawn with a 0.5 m near plane, then depth is cleared and the cockpit
is drawn with a 2 cm near plane, so instruments close to the eye neither clip nor fight with
distant terrain for depth precision. The default horizontal field of view is 90°.

## Materials

Cockpits use untextured layers (`tfNoTexture 1`) that draw their `ColorScale` colour. An example
is the `Mask` material: black with zero alpha and blending on, which hides geometry without
being seen. See `docs/formats/mat.md`.

## OpenIL2 controls

F1 cockpit, F2 free orbit camera, F3 chase camera. In the cockpit, dragging with the left mouse
button turns the head (up to 150° either side, 35° down, 80° up). These limits are OpenIL2's own.
