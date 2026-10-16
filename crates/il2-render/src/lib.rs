//! Rendering for OpenIL2: wgpu renderer, cameras, and loading of game assets into GPU-ready data.

pub mod camera;
pub mod render;
pub mod scene;

pub use render::{Batch, BlendMode, Environment, GpuMesh, GpuTerrain, Image, InstanceRaw, InstancedDraw, MeshData, Renderer, TerrainData, Vertex};
