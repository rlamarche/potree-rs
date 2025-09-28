#[derive(Debug, Clone)]
pub struct PointAttributeType {
    pub ordinal: usize,
    pub name: &'static str,
    pub size: u16,
}

macro_rules! create_data_type {
    // The `tt` (token tree) designator is used for
    // operators and tokens.
    ($const_name:ident, $ord:expr, $name:expr, $size:expr) => {
        pub static $const_name: PointAttributeType = PointAttributeType {
            ordinal: $ord,
            name: $name,
            size: $size,
        };
    };
}

create_data_type!(DATA_TYPE_DOUBLE, 0, "double", 8);
create_data_type!(DATA_TYPE_FLOAT, 1, "float", 4);
create_data_type!(DATA_TYPE_INT8, 2, "int8", 1);
create_data_type!(DATA_TYPE_UINT8, 3, "uint8", 1);
create_data_type!(DATA_TYPE_INT16, 4, "int16", 2);
create_data_type!(DATA_TYPE_UINT16, 5, "uint16", 2);
create_data_type!(DATA_TYPE_INT32, 4, "int32", 4);
create_data_type!(DATA_TYPE_UINT32, 5, "uint32", 4);
create_data_type!(DATA_TYPE_INT64, 4, "int64", 8);
create_data_type!(DATA_TYPE_UINT64, 5, "uint64", 8);

pub const POINT_ATTRIBUTE_TYPES: &[&PointAttributeType] = &[
    &DATA_TYPE_DOUBLE,
    &DATA_TYPE_FLOAT,
    &DATA_TYPE_INT8,
    &DATA_TYPE_UINT8,
    &DATA_TYPE_INT16,
    &DATA_TYPE_UINT16,
    &DATA_TYPE_INT32,
    &DATA_TYPE_UINT32,
    &DATA_TYPE_INT64,
    &DATA_TYPE_UINT64,
];

#[derive(Debug, Clone)]
pub struct PointAttribute {
    pub name: String,
    pub r#type: &'static PointAttributeType,
    pub num_elements: u16,
    pub byte_size: u16,
    pub description: String,
    pub range: (f32, f32),
}

#[derive(Debug, Clone)]
pub struct PointAttributes {
    pub attributes: Vec<PointAttribute>,
    pub byte_size: u16,
    pub size: u16,
    pub vectors: Vec<PointVector>,
}

#[derive(Debug, Clone)]
pub struct PointVector {
    pub name: String,
    pub attributes: Vec<String>,
}
