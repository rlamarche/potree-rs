use glam::{DVec3, U8Vec3};

#[derive(Clone, Debug, Default)]
pub struct PointData {
    pub position: DVec3,
    pub color: U8Vec3,
}
