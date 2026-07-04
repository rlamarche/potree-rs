use std::sync::Arc;

#[derive(Clone, Debug, Copy, Eq, PartialEq)]
pub enum AttributeType {
    Position,
    Rgb,
    Intensity,
    ReturnNumber,
    NumberOfReturns,
    Classification,
    ScanAngleRank,
    PointSourceId,
    Normal,
    NormalX,
    NormalY,
    NormalZ,
    UserData,
}

impl From<&str> for AttributeType {
    fn from(value: &str) -> Self {
        match value {
            "POSITION_CARTESIAN" | "position" => Self::Position,
            "RGBA" | "rgba" | "RGB" | "rgb" => Self::Rgb,
            "intensity" => Self::Intensity,
            "return number" => Self::ReturnNumber,
            "number of returns" => Self::NumberOfReturns,
            "classification" => Self::Classification,
            "scan angle rank" => Self::ScanAngleRank,
            "point source id" => Self::PointSourceId,
            "normal" => Self::Normal,
            "normal x" => Self::NormalX,
            "normal y" => Self::NormalY,
            "normal z" => Self::NormalZ,
            _ => Self::UserData,
        }
    }
}

/// Attribute metadata
#[derive(Clone, Debug)]
pub struct AttributeInfo {
    pub name: String,
    pub r#type: AttributeType,
    pub offset: usize,
    pub stride: usize,
}

pub type AttributeLayout = Vec<AttributeInfo>;

#[derive(Clone, Debug, Default)]
pub struct PointBuffer {
    pub count: usize,
    pub stride: usize,
    pub attributes: Vec<f32>, // count * attributes_stride_total
    pub layout: Arc<AttributeLayout>,
}

impl PointBuffer {
    pub fn new(layout: AttributeLayout) -> Self {
        let stride: usize = layout
            .iter()
            .map(|attribute_info| attribute_info.stride)
            .sum();

        Self {
            count: 0,
            stride,
            attributes: Vec::new(),
            layout: layout.into(),
        }
    }

    pub fn with_size(size: usize, layout: AttributeLayout) -> Self {
        let stride: usize = layout.iter().map(|a| a.stride).sum();

        Self {
            count: size,
            stride,
            attributes: vec![0.0; stride * size],
            layout: layout.into(),
        }
    }

    /// Append one point.
    ///
    /// `attrs` – f32 values in the **same order** attributes were registered
    ///
    /// # Panics
    /// Panics in debug mode if `attrs.len() != self.stride`.
    #[inline]
    pub fn push(&mut self, attrs: &[f32]) {
        debug_assert_eq!(
            attrs.len(),
            self.stride,
            "attribute slice length must equal stride ({})",
            self.stride
        );
        self.attributes.extend_from_slice(attrs);
    }

    /// Slice on all attributes, for 1 points
    pub fn get(&self, i: usize) -> Option<PointSlice<'_>> {
        if self.attributes.len() >= (i + 1) * self.stride {
            Some(PointSlice {
                data: &self.attributes[i * self.stride..(i + 1) * self.stride],
                layout: &self.layout,
            })
        } else {
            None
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = PointSlice<'_>> {
        self.attributes
            .chunks_exact(self.stride)
            .take(self.count)
            .map(|chunk| PointSlice {
                data: chunk,
                layout: &self.layout,
            })
    }

    /// Slice on 1 attribute, for all points
    pub fn attribute_slice(&self, name: &str) -> Option<AttributeSlice<'_>> {
        let info = self.layout.iter().find(|a| a.name == name)?;
        Some(AttributeSlice {
            data: &self.attributes,
            offset: info.offset,
            stride: self.stride,
            count: self.count,
            width: info.stride,
        })
    }

    /// Slice on 1 attribute, for all points, with mutable access
    pub fn attribute_slice_mut(&mut self, name: &str) -> Option<AttributeSliceMut<'_>> {
        let info = self.layout.iter().find(|a| a.name == name)?;
        Some(AttributeSliceMut {
            data: &mut self.attributes,
            offset: info.offset,
            stride: self.stride,
            count: self.count,
            width: info.stride,
        })
    }
}

/// Iterator/accessor on attributes
pub struct PointSlice<'a> {
    pub data: &'a [f32],
    pub layout: &'a AttributeLayout,
}

impl<'a> PointSlice<'a> {
    pub fn attribute(&self, name: &str) -> Option<&[f32]> {
        let attribute_info = self.layout.iter().find(|a| a.name.eq(name))?;

        Some(&self.data[attribute_info.offset..(attribute_info.offset + attribute_info.stride)])
    }
    pub fn attribute_type(&self, attribute_type: AttributeType) -> Option<&[f32]> {
        let attribute_info = self.layout.iter().find(|a| a.r#type.eq(&attribute_type))?;

        Some(&self.data[attribute_info.offset..(attribute_info.offset + attribute_info.stride)])
    }
}

/// Iterator/accessor on attributes
pub struct AttributeSlice<'a> {
    data: &'a [f32],
    offset: usize,
    stride: usize,
    count: usize,
    width: usize,
}

impl<'a> AttributeSlice<'a> {
    /// Get nth point
    #[inline]
    pub fn get(&self, i: usize) -> &[f32] {
        let start = i * self.stride + self.offset;
        &self.data[start..start + self.width]
    }

    /// Iterator on all points
    pub fn iter(&self) -> impl Iterator<Item = &[f32]> {
        (0..self.count).map(move |i| self.get(i))
    }
}

/// Iterator/accessor on attributes
pub struct AttributeSliceMut<'a> {
    data: &'a mut [f32],
    offset: usize,
    stride: usize,
    count: usize,
    width: usize,
}

impl<'a> AttributeSliceMut<'a> {
    /// Get nth point
    #[inline]
    pub fn get(&self, i: usize) -> &[f32] {
        let start = i * self.stride + self.offset;
        &self.data[start..start + self.width]
    }

    /// Get nth point
    #[inline]
    pub fn get_mut(&mut self, i: usize) -> &mut [f32] {
        let start = i * self.stride + self.offset;
        &mut self.data[start..start + self.width]
    }

    pub fn iter(&self) -> impl Iterator<Item = &[f32]> {
        self.data
            .chunks_exact(self.stride)
            .take(self.count)
            .map(|chunk| &chunk[self.offset..self.offset + self.width])
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut [f32]> {
        self.data
            .chunks_exact_mut(self.stride)
            .map(|chunk| &mut chunk[self.offset..self.offset + self.width])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xyz_layout() -> AttributeLayout {
        vec![AttributeInfo {
            name: "position".to_string(),
            r#type: AttributeType::Position,
            offset: 0,
            stride: 3,
        }]
    }

    #[test]
    fn get_returns_every_point_including_the_last() {
        let mut buffer = PointBuffer::new(xyz_layout());
        buffer.push(&[1.0, 2.0, 3.0]);
        buffer.push(&[4.0, 5.0, 6.0]);
        buffer.count = 2;

        assert_eq!(buffer.get(0).unwrap().data, &[1.0, 2.0, 3.0]);
        assert_eq!(
            buffer.get(1).expect("last point must be accessible").data,
            &[4.0, 5.0, 6.0]
        );
        assert!(buffer.get(2).is_none());
    }
}
