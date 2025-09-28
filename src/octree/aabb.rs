use glam::DVec3;

#[derive(Clone, Debug, Default)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub fn new(min: DVec3, max: DVec3) -> Self {
        Self { min, max }
    }
}

pub fn create_child_aabb(aabb: &Aabb, index: usize) -> Aabb {
    let mut min = aabb.min;
    let mut max = aabb.max;
    let size = (max - min) * 0.5;

    if (index & 0b0001) > 0 {
        min.z += size.z;
    } else {
        max.z -= size.z;
    }
    if (index & 0b0010) > 0 {
        min.y += size.y;
    } else {
        max.y -= size.y;
    }
    if (index & 0b0100) > 0 {
        min.x += size.x;
    } else {
        max.x -= size.x;
    }

    Aabb::new(min, max)
}
