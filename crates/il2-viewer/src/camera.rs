use glam::{Mat4, Vec3};

/// Orbit camera around a target point. Game coordinates are Z-up.
pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl OrbitCamera {
    pub fn new(radius: f32) -> Self {
        Self { target: Vec3::ZERO, distance: radius * 1.5, yaw: 0.8, pitch: 0.35 }
    }

    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(cy * cp, sy * cp, sp) * self.distance
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye(), self.target, Vec3::Z);
        let near = (self.distance * 0.01).max(0.01);
        let proj = Mat4::perspective_rh(50f32.to_radians(), aspect, near, self.distance * 100.0);
        proj * view
    }

    pub fn rotate(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.55, 1.55);
    }

    pub fn zoom(&mut self, steps: f32) {
        self.distance *= 0.9f32.powf(steps);
    }
}
