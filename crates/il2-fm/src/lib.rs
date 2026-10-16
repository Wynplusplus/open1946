//! Aircraft flight model. Behavior spec: `docs/behavior/flight-model.md`.

pub mod aircraft;
pub mod atmosphere;
pub mod damage;
pub mod engine;
pub mod gear;
pub mod polar;
pub mod sim;

pub use aircraft::{Aircraft, ContactKind, GearPoint};
pub use sim::{Ground, Inputs, Sim, TICK};
