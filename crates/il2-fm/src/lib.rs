//! Aircraft flight model. Behavior spec: `docs/behavior/flight-model.md`.

pub mod aircraft;
pub mod atmosphere;
pub mod polar;
pub mod sim;

pub use aircraft::{Aircraft, GearPoint};
pub use sim::{Ground, Inputs, Sim, TICK};
