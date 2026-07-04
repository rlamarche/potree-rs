use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use ply_rs_bw::{
    parser::Parser,
    ply::{DefaultElement, Ply, Property},
};
use thiserror::Error;

use crate::convert::buffer::PotreeBuilder;

#[derive(Debug, Error)]
pub enum PlyLoadError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ply parse error: {0}")]
    Parse(String),

    #[error("ply missing vertex element")]
    MissingVertex,

    #[error("ply missing x/y/z properties")]
    MissingPosition,
}

#[derive(Debug, Clone)]
pub struct PlyPositions {
    pub positions: Vec<[f64; 3]>,
    pub colors: Option<Vec<[u16; 3]>>,
}

impl PlyPositions {
    pub fn into_potree_builder(self) -> PotreeBuilder {
        let mut builder = PotreeBuilder::new().positions(self.positions);
        if let Some(colors) = self.colors {
            builder = builder.colors(colors);
        }
        builder
    }
}

pub fn load_ply_positions(path: &Path) -> Result<PlyPositions, PlyLoadError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let parser = Parser::<DefaultElement>::new();
    let ply = parser
        .read_ply(&mut reader)
        .map_err(|err| PlyLoadError::Parse(err.to_string()))?;

    extract_positions(&ply)
}

fn extract_positions(ply: &Ply<DefaultElement>) -> Result<PlyPositions, PlyLoadError> {
    let Some(vertex) = ply.payload.get("vertex") else {
        return Err(PlyLoadError::MissingVertex);
    };

    let mut positions = Vec::with_capacity(vertex.len());
    let mut colors: Option<Vec<[u16; 3]>> = None;

    for element in vertex {
        let x = prop_as_f64(element.get("x").ok_or(PlyLoadError::MissingPosition)?)?;
        let y = prop_as_f64(element.get("y").ok_or(PlyLoadError::MissingPosition)?)?;
        let z = prop_as_f64(element.get("z").ok_or(PlyLoadError::MissingPosition)?)?;

        positions.push([x, y, z]);

        match extract_color(element) {
            Ok(Some(color)) => match &mut colors {
                Some(c) => c.push(color),
                None => {
                    let mut c = Vec::with_capacity(vertex.len());
                    c.push(color);
                    colors = Some(c);
                }
            },
            Ok(None) => {}
            Err(e) => {
                eprintln!("Error extracting color: {e}");
                // If there's an error extracting color, should we discard all colors?
            }
        }
    }

    Ok(PlyPositions { positions, colors })
}

fn prop_as_f64(prop: &Property) -> Result<f64, PlyLoadError> {
    match prop {
        Property::Char(v) => Ok(*v as f64),
        Property::UChar(v) => Ok(*v as f64),
        Property::Short(v) => Ok(*v as f64),
        Property::UShort(v) => Ok(*v as f64),
        Property::Int(v) => Ok(*v as f64),
        Property::UInt(v) => Ok(*v as f64),
        Property::Float(v) => Ok(*v as f64),
        Property::Double(v) => Ok(*v),
        Property::ListInt(_) => Err(PlyLoadError::MissingPosition),
        Property::ListUInt(_) => Err(PlyLoadError::MissingPosition),
        Property::ListFloat(_) => Err(PlyLoadError::MissingPosition),
        Property::ListDouble(_) => Err(PlyLoadError::MissingPosition),
        Property::ListChar(_) => Err(PlyLoadError::MissingPosition),
        Property::ListUChar(_) => Err(PlyLoadError::MissingPosition),
        Property::ListShort(_) => Err(PlyLoadError::MissingPosition),
        Property::ListUShort(_) => Err(PlyLoadError::MissingPosition),
    }
}

fn extract_color(element: &DefaultElement) -> Result<Option<[u16; 3]>, PlyLoadError> {
    let r = element.get("red").or_else(|| element.get("r"));
    let g = element.get("green").or_else(|| element.get("g"));
    let b = element.get("blue").or_else(|| element.get("b"));

    let (Some(r), Some(g), Some(b)) = (r, g, b) else {
        return Ok(None);
    };

    let r = prop_as_u16_color(r)?;
    let g = prop_as_u16_color(g)?;
    let b = prop_as_u16_color(b)?;

    Ok(Some([r, g, b]))
}

fn prop_as_u16_color(prop: &Property) -> Result<u16, PlyLoadError> {
    match prop {
        Property::UChar(v) => Ok((*v as u16) * 257),
        Property::Char(v) => Ok((*v as i16).clamp(0, 255) as u16 * 257),
        Property::UShort(v) => Ok(*v),
        Property::Short(v) => Ok((*v as i32).clamp(0, u16::MAX as i32) as u16),
        Property::UInt(v) => Ok((*v).min(u16::MAX as u32) as u16),
        Property::Int(v) => Ok((*v).clamp(0, u16::MAX as i32) as u16),
        Property::Float(v) => Ok((*v as f64).round().clamp(0.0, u16::MAX as f64) as u16),
        Property::Double(v) => Ok((*v).round().clamp(0.0, u16::MAX as f64) as u16),
        _ => Err(PlyLoadError::MissingPosition),
    }
}
