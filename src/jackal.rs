// Jackal compression format.
//
// It is hybrid compression algorithm designed to work on blocks that have
// color data and indices.
// Color data is compressed using combination of run-length, hash and diff encoding.
// Indices are compressed by LZ77 algorithm with parameters predefined for each block format.
//
// MOK format compresses super-blocks (blocks of blocks) independently.
// This allows parallel processing of super-blocks on multi-core CPU and GPU.
// Although small textures may have just one super-block.

use std::{
    io::{Read, Seek, SeekFrom, Write},
    u32,
};

use crate::{
    bc1,
    bytes::LeBytes,
    lz78,
    math::{predict_color_u8, PredictableColor, Rgb565},
};

#[derive(Clone, Copy, Debug)]
pub enum DecodeError {
    /// Header is invalid.
    /// Data is corrupted.
    InvalidHeader,

    // Unsupported format.
    // Format id is not recognized.
    UnsupportedFormat,

    /// Texture extent value is invalid.
    /// For example 1D texture have height or depth.
    InvalidExtent,

    // Data is invalid.
    // Such as position is out of bounds.
    InvalidData,
}

/// Size of the super-block in number of blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Footprint(u16, u16);

fn footprint_from_size(size: u32) -> u16 {
    return 1024;
    match size {
        0 => unreachable!(),
        1..64 => 16,
        64..256 => 32,
        256..1024 => 64,
        1024..4096 => 128,
        _ => 256,
    }
}

impl Footprint {
    const COUNT: u32 = 25;

    fn encode(&self) -> u32 {
        return 0;
        let size = |x| match x {
            16 => 0,
            32 => 1,
            64 => 2,
            128 => 3,
            256 => 4,
            _ => unreachable!(),
        };

        size(self.0) * 5 + size(self.1)
    }

    fn decode(id: u32) -> Self {
        assert!(id < Self::COUNT);
        return Footprint(1024, 1024);

        let size = |x| match x {
            0 => 16,
            1 => 32,
            2 => 64,
            3 => 128,
            4 => 256,
            _ => unreachable!(),
        };

        Footprint(size(id / 5), size(id % 5))
    }

    fn from_size(width: u32, height: u32) -> Self {
        Footprint(footprint_from_size(width), footprint_from_size(height))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    BC1,
    BC3,
    BC4,
    BC5,
    BC6,
    BC7,
    // Entries to add new format support.
    // Unused6,
    // Unused7,
    // Unused8,
    // Unused9,
    // No more entries can be added.
}

impl Format {
    const COUNT: u32 = 10;

    fn encode(&self) -> u32 {
        match self {
            Format::BC1 => 0,
            Format::BC3 => 1,
            Format::BC4 => 2,
            Format::BC5 => 3,
            Format::BC6 => 4,
            Format::BC7 => 5,
        }
    }

    fn decode(id: u32) -> Result<Self, DecodeError> {
        assert!(id < Self::COUNT);

        match id {
            0 => Ok(Format::BC1),
            1 => Ok(Format::BC3),
            2 => Ok(Format::BC4),
            3 => Ok(Format::BC5),
            4 => Ok(Format::BC6),
            5 => Ok(Format::BC7),
            6 => Err(DecodeError::UnsupportedFormat),
            7 => Err(DecodeError::UnsupportedFormat),
            8 => Err(DecodeError::UnsupportedFormat),
            9 => Err(DecodeError::UnsupportedFormat),
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dimensions {
    D1,
    D2,
    D3,
    D1Array,
    D2Array,
}

impl Dimensions {
    const COUNT: u32 = 5;

    fn encode(&self) -> u32 {
        match self {
            Dimensions::D1 => 0,
            Dimensions::D2 => 1,
            Dimensions::D3 => 2,
            Dimensions::D1Array => 3,
            Dimensions::D2Array => 4,
        }
    }

    fn decode(id: u32) -> Self {
        assert!(id < Self::COUNT);

        match id {
            0 => Dimensions::D1,
            1 => Dimensions::D2,
            2 => Dimensions::D3,
            3 => Dimensions::D1Array,
            4 => Dimensions::D2Array,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
struct MipLevels(u8);

impl MipLevels {
    const COUNT: u32 = 30;

    fn encode(&self) -> u32 {
        self.0 as u32
    }

    fn decode(id: u32) -> Self {
        assert!(id < Self::COUNT);

        MipLevels(id as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Config {
    // Dimensions of the texture.
    dimensions: Dimensions,

    // Number of texture mip levels.
    levels: MipLevels,

    // Format of the blocks.
    format: Format,

    // Footprint of super-blocks.
    footprint: Footprint,
}

impl Config {
    fn encode(&self) -> u32 {
        let levels = self.levels.encode();
        let dimensions = self.dimensions.encode();
        let footprint = self.footprint.encode();
        let format = self.format.encode();

        let mut config = 0;

        config *= MipLevels::COUNT;
        config += levels;

        config *= Dimensions::COUNT;
        config += dimensions;

        config *= Footprint::COUNT;
        config += footprint;

        config *= Format::COUNT;
        config += format;

        config
    }

    fn decode(config: u32) -> Result<Self, DecodeError> {
        let mut config = config;

        let format = Format::decode(config % Format::COUNT)?;
        config /= Format::COUNT;

        let footprint = Footprint::decode(config % Footprint::COUNT);
        config /= Footprint::COUNT;

        let dimensions = Dimensions::decode(config % Dimensions::COUNT);
        config /= Dimensions::COUNT;

        let levels = MipLevels::decode(config);
        config /= MipLevels::COUNT;

        let _ = config;

        Ok(Config {
            dimensions,
            levels,
            format,
            footprint,
        })
    }
}

const MAGIC_VER_0: u32 = 0x304C4B4Au32; // "JKL0"

#[derive(Clone, Copy)]
pub struct JackalHeader {
    // Number of texture mip levels.
    levels: MipLevels,

    // Format of the blocks.
    format: Format,

    // Footprint of super-blocks.
    footprint: Footprint,

    /// Extent of the image. Decoded based on dimensions.
    extent: Extent,
}

impl JackalHeader {
    const BYTES_SIZE: usize = 20;

    fn write_to(&self, mut write: impl Write) -> std::io::Result<()> {
        let dimensions = self.extent.dimensions();
        let raw_size = self.extent.raw_size();

        let config = Config {
            dimensions,
            levels: self.levels,
            format: self.format,
            footprint: self.footprint,
        };

        let mut bytes = [0; Self::BYTES_SIZE];
        bytes[0..4].copy_from_slice(&MAGIC_VER_0.to_le_bytes());
        bytes[4..8].copy_from_slice(&config.encode().to_le_bytes());
        bytes[8..12].copy_from_slice(&raw_size[0].to_le_bytes());
        bytes[12..16].copy_from_slice(&raw_size[1].to_le_bytes());
        bytes[16..20].copy_from_slice(&raw_size[2].to_le_bytes());
        write.write_all(&bytes)
    }

    fn read_from(mut read: impl Read) -> Result<Self, DecompressError> {
        let mut bytes = [0; Self::BYTES_SIZE];
        read.read_exact(&mut bytes)?;

        let mut magic_bytes = [0; 4];
        magic_bytes.copy_from_slice(&bytes[0..4]);
        let magic = u32::from_le_bytes(magic_bytes);
        if magic != MAGIC_VER_0 {
            return Err(DecodeError::InvalidHeader.into());
        }

        let mut config_bytes = [0; 4];
        config_bytes.copy_from_slice(&bytes[4..8]);
        let config = Config::decode(u32::from_le_bytes(config_bytes))?;

        let mut extent_bytes = [0; 4];
        extent_bytes.copy_from_slice(&bytes[8..12]);
        let width = u32::from_le_bytes(extent_bytes);
        extent_bytes.copy_from_slice(&bytes[12..16]);
        let height = u32::from_le_bytes(extent_bytes);
        extent_bytes.copy_from_slice(&bytes[16..20]);
        let depth = u32::from_le_bytes(extent_bytes);

        let raw_size = [width, height, depth];
        let extent = Extent::from_raw_size(raw_size, config.dimensions)?;

        Ok(JackalHeader {
            levels: config.levels,
            format: config.format,
            footprint: config.footprint,
            extent,
        })
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }

    pub fn jackal_blocks_count(&self) -> usize {
        let [width, height, depth] = self.jackal_blocks_extent();
        (width * height * depth) as usize
    }

    pub fn jackal_blocks_extent(&self) -> [u32; 3] {
        let raw_size = self.extent.raw_size();
        let jackal_blocks_width =
            (raw_size[0] + self.footprint.0 as u32 - 1) / self.footprint.0 as u32;
        let jackal_blocks_height =
            (raw_size[1] + self.footprint.1 as u32 - 1) / self.footprint.1 as u32;
        let jackal_blocks_depth = raw_size[2];

        [
            jackal_blocks_width,
            jackal_blocks_height,
            jackal_blocks_depth,
        ]
    }

    pub fn blocks_count(&self) -> usize {
        let raw_size = self.extent.raw_size();
        raw_size[0] as usize * raw_size[1] as usize * raw_size[2] as usize
    }
}

#[derive(Clone, Copy)]
pub struct JackalBlock {
    offset: u64,
}

impl JackalBlock {
    const BYTES_SIZE: usize = 8;

    fn write_to(&self, mut write: impl Write) -> std::io::Result<()> {
        write.write_all(&self.offset.to_le_bytes())
    }

    fn read_from(mut read: impl Read) -> Result<Self, DecompressError> {
        let mut bytes = [0; Self::BYTES_SIZE];
        read.read_exact(&mut bytes)?;
        let offset = u64::from_le_bytes(bytes);
        Ok(JackalBlock { offset })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    D1 {
        width: u32,
    },
    D2 {
        width: u32,
        height: u32,
    },
    D3 {
        width: u32,
        height: u32,
        depth: u32,
    },
    D1Array {
        width: u32,
        layers: u32,
    },
    D2Array {
        width: u32,
        height: u32,
        layers: u32,
    },
}

impl Extent {
    fn width(&self) -> u32 {
        match *self {
            Extent::D1 { width } => width,
            Extent::D2 { width, .. } => width,
            Extent::D3 { width, .. } => width,
            Extent::D1Array { width, .. } => width,
            Extent::D2Array { width, .. } => width,
        }
    }

    fn height(&self) -> u32 {
        match *self {
            Extent::D1 { .. } => 1,
            Extent::D2 { height, .. } => height,
            Extent::D3 { height, .. } => height,
            Extent::D1Array { .. } => 1,
            Extent::D2Array { height, .. } => height,
        }
    }

    fn depth(&self) -> u32 {
        match *self {
            Extent::D1 { .. } => 1,
            Extent::D2 { .. } => 1,
            Extent::D3 { depth, .. } => depth,
            Extent::D1Array { .. } => 1,
            Extent::D2Array { .. } => 1,
        }
    }

    fn layers(&self) -> u32 {
        match *self {
            Extent::D1 { .. } => 1,
            Extent::D2 { .. } => 1,
            Extent::D3 { .. } => 1,
            Extent::D1Array { layers, .. } => layers,
            Extent::D2Array { layers, .. } => layers,
        }
    }

    fn dimensions(self) -> Dimensions {
        match self {
            Extent::D1 { .. } => Dimensions::D1,
            Extent::D2 { .. } => Dimensions::D2,
            Extent::D3 { .. } => Dimensions::D3,
            Extent::D1Array { .. } => Dimensions::D1Array,
            Extent::D2Array { .. } => Dimensions::D2Array,
        }
    }

    fn raw_size(self) -> [u32; 3] {
        match self {
            Extent::D1 { width } => [width, 1, 1],
            Extent::D2 { width, height } => [width, height, 1],
            Extent::D3 {
                width,
                height,
                depth,
            } => [width, height, depth],
            Extent::D1Array { width, layers } => [width, layers, 1],
            Extent::D2Array {
                width,
                height,
                layers,
            } => [width, height, layers],
        }
    }

    fn from_raw_size(value: [u32; 3], dimensions: Dimensions) -> Result<Self, DecodeError> {
        match dimensions {
            Dimensions::D1 => {
                if value[1] != 1 || value[2] != 1 {
                    return Err(DecodeError::InvalidExtent);
                }
                Ok(Extent::D1 { width: value[0] })
            }
            Dimensions::D2 => {
                if value[2] != 1 {
                    return Err(DecodeError::InvalidExtent);
                }
                Ok(Extent::D2 {
                    width: value[0],
                    height: value[1],
                })
            }
            Dimensions::D3 => Ok(Extent::D3 {
                width: value[0],
                height: value[1],
                depth: value[2],
            }),
            Dimensions::D1Array => {
                if value[2] != 1 {
                    return Err(DecodeError::InvalidExtent);
                }
                Ok(Extent::D1Array {
                    width: value[0],
                    layers: value[1],
                })
            }
            Dimensions::D2Array => Ok(Extent::D2Array {
                width: value[0],
                height: value[1],
                layers: value[2],
            }),
        }
    }
}

trait AnyBlock: Copy + 'static + Sized {
    const ASPECTS: usize;

    type EncoderElement: Copy + Eq + LeBytes;

    /// Compress one block aspect.
    fn compress<'a, const ASPECT: usize>(
        &self,
        left: Option<&'a Self>,
        top: Option<&'a Self>,
        top_left: Option<&'a Self>,
        encoder: &mut lz78::Encoder<Self::EncoderElement>,
        write: &mut impl Write,
    ) -> std::io::Result<()>;

    /// Decompress one block aspect.
    fn decompress<'a, const ASPECT: usize>(
        &mut self,
        left: Option<&'a Self>,
        top: Option<&'a Self>,
        top_left: Option<&'a Self>,
        decoder: &mut lz78::Decoder<Self::EncoderElement>,
        read: &mut impl Read,
    ) -> Result<(), DecompressError>;
}

impl AnyBlock for bc1::Block {
    const ASPECTS: usize = 7;
    type EncoderElement = u8;

    fn compress<'a, const ASPECT: usize>(
        &self,
        left: Option<&'a Self>,
        top: Option<&'a Self>,
        top_left: Option<&'a Self>,
        encoder: &mut lz78::Encoder<u8>,
        write: &mut impl Write,
    ) -> std::io::Result<()> {
        match ASPECT {
            0 => {
                // Color0-red
                let left = left.map_or(0, |b| b.color0.r());
                let top = top.map_or(0, |b| b.color0.r());
                let top_left = top_left.map_or(0, |b| b.color0.r());

                let predicted_color0r = predict_color_u8(left, top, top_left);
                let diff_color0r = u8::wrapping_sub(self.color0.r(), predicted_color0r);

                encoder.encode(diff_color0r, write)?;
            }
            1 => {
                // Color0-green
                let left = left.map_or(0, |b| b.color0.g());
                let top = top.map_or(0, |b| b.color0.g());
                let top_left = top_left.map_or(0, |b| b.color0.g());

                let predicted_color0g = predict_color_u8(left, top, top_left);
                let diff_color0g = u8::wrapping_sub(self.color0.g(), predicted_color0g);

                encoder.encode(diff_color0g, write)?;
            }
            2 => {
                // Color0-blue
                let left = left.map_or(0, |b| b.color0.b());
                let top = top.map_or(0, |b| b.color0.b());
                let top_left = top_left.map_or(0, |b| b.color0.b());

                let predicted_color0b = predict_color_u8(left, top, top_left);
                let diff_color0b = u8::wrapping_sub(self.color0.b(), predicted_color0b);

                encoder.encode(diff_color0b, write)?;
            }
            3 => {
                // Color1-red
                let left = left.map_or(0, |b| b.color1.r());
                let top = top.map_or(0, |b| b.color1.r());
                let top_left = top_left.map_or(0, |b| b.color1.r());

                let predicted_color1r = predict_color_u8(left, top, top_left);
                let diff_color1r = u8::wrapping_sub(self.color1.r(), predicted_color1r);

                encoder.encode(diff_color1r, write)?;
            }
            4 => {
                // Color1-green
                let left = left.map_or(0, |b| b.color1.g());
                let top = top.map_or(0, |b| b.color1.g());
                let top_left = top_left.map_or(0, |b| b.color1.g());

                let predicted_color1g = predict_color_u8(left, top, top_left);
                let diff_color1g = u8::wrapping_sub(self.color1.g(), predicted_color1g);

                encoder.encode(diff_color1g, write)?;
            }
            5 => {
                // Color1-blue
                let left = left.map_or(0, |b| b.color1.b());
                let top = top.map_or(0, |b| b.color1.b());
                let top_left = top_left.map_or(0, |b| b.color1.b());

                let predicted_color1b = predict_color_u8(left, top, top_left);
                let diff_color1b = u8::wrapping_sub(self.color1.b(), predicted_color1b);

                encoder.encode(diff_color1b, write)?;
            }
            6 => {
                // Texels
                encoder.encode(self.texels[0], write)?;
                encoder.encode(self.texels[1], write)?;
                encoder.encode(self.texels[2], write)?;
                encoder.encode(self.texels[3], write)?;
            }
            _ => unreachable!(),
        }

        Ok(())
    }

    fn decompress<'a, const ASPECT: usize>(
        &mut self,
        left: Option<&'a Self>,
        top: Option<&'a Self>,
        top_left: Option<&'a Self>,
        decoder: &mut lz78::Decoder<u8>,
        read: &mut impl Read,
    ) -> Result<(), DecompressError> {
        match ASPECT {
            0 => {
                // Color0-red
                let left = left.map_or(0, |b| b.color0.r());
                let top = top.map_or(0, |b| b.color0.r());
                let top_left = top_left.map_or(0, |b| b.color0.r());

                let predicted_color0r = predict_color_u8(left, top, top_left);

                let diff_color0r = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;

                self.color0
                    .set_r(u8::wrapping_add(diff_color0r, predicted_color0r));
            }
            1 => {
                // Color0-green
                let left = left.map_or(0, |b| b.color0.g());
                let top = top.map_or(0, |b| b.color0.g());
                let top_left = top_left.map_or(0, |b| b.color0.g());

                let predicted_color0g = predict_color_u8(left, top, top_left);

                let diff_color0g = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;

                self.color0
                    .set_g(u8::wrapping_add(diff_color0g, predicted_color0g));
            }
            2 => {
                // Color0-blue
                let left = left.map_or(0, |b| b.color0.b());
                let top = top.map_or(0, |b| b.color0.b());
                let top_left = top_left.map_or(0, |b| b.color0.b());

                let predicted_color0b = predict_color_u8(left, top, top_left);

                let diff_color0b = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;

                self.color0
                    .set_b(u8::wrapping_add(diff_color0b, predicted_color0b));
            }
            3 => {
                // Color1-red
                let left = left.map_or(0, |b| b.color1.r());
                let top = top.map_or(0, |b| b.color1.r());
                let top_left = top_left.map_or(0, |b| b.color1.r());

                let predicted_color1r = predict_color_u8(left, top, top_left);

                let diff_color1r = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;

                self.color1
                    .set_r(u8::wrapping_add(diff_color1r, predicted_color1r));
            }
            4 => {
                // Color1-green
                let left = left.map_or(0, |b| b.color1.g());
                let top = top.map_or(0, |b| b.color1.g());
                let top_left = top_left.map_or(0, |b| b.color1.g());

                let predicted_color1g = predict_color_u8(left, top, top_left);

                let diff_color1g = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;

                self.color1
                    .set_g(u8::wrapping_add(diff_color1g, predicted_color1g));
            }
            5 => {
                // Color1-blue
                let left = left.map_or(0, |b| b.color1.b());
                let top = top.map_or(0, |b| b.color1.b());
                let top_left = top_left.map_or(0, |b| b.color1.b());

                let predicted_color1b = predict_color_u8(left, top, top_left);

                let diff_color1b = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;

                self.color1
                    .set_b(u8::wrapping_add(diff_color1b, predicted_color1b));
            }
            6 => {
                // Texels
                self.texels[0] = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;
                self.texels[1] = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;
                self.texels[2] = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;
                self.texels[3] = *decoder
                    .decode_next(read)
                    .map_err(lz78_decode_to_decompress_error)?;
            }
            _ => unreachable!(),
        }

        Ok(())
    }
}

pub fn compress_bc1_texture(
    extent: Extent,
    blocks: &[bc1::Block],
    write: (impl Write + Seek),
) -> std::io::Result<()> {
    compress_texture(extent, blocks, write)
}

fn compress_texture<B>(
    extent: Extent,
    blocks: &[B],
    mut write: (impl Write + Seek),
) -> std::io::Result<()>
where
    B: AnyBlock,
{
    let raw_size = extent.raw_size();

    assert_eq!(blocks.len() as u32, raw_size[0] * raw_size[1] * raw_size[2]);

    let footprint = Footprint::from_size(raw_size[0], raw_size[1]);

    let header = JackalHeader {
        levels: MipLevels(1),
        format: Format::BC1,
        footprint,
        extent,
    };

    let start = write.seek(SeekFrom::Current(0))?;
    header.write_to(&mut write)?;

    let jackal_blocks_width = (raw_size[0] + footprint.0 as u32 - 1) / footprint.0 as u32;
    let jackal_blocks_height = (raw_size[1] + footprint.1 as u32 - 1) / footprint.1 as u32;
    let jackal_blocks_depth = raw_size[2];

    let jackal_blocks_count = jackal_blocks_width * jackal_blocks_height * jackal_blocks_depth;

    let jackal_blocks_start = start + JackalHeader::BYTES_SIZE as u64;
    let jackal_blocks_end =
        jackal_blocks_start + JackalBlock::BYTES_SIZE as u64 * jackal_blocks_count as u64;

    let mut next_jackal_block_pos = jackal_blocks_start;
    let mut next_data_pos = jackal_blocks_end;

    for z in 0..raw_size[2] {
        for y_start in (0..raw_size[1]).step_by(footprint.1 as usize) {
            let y_end = if raw_size[1] - y_start < header.footprint.1 as u32 {
                raw_size[1]
            } else {
                y_start + header.footprint.1 as u32
            };

            for x_start in (0..raw_size[0]).step_by(footprint.0 as usize) {
                let x_end = if raw_size[0] - x_start < header.footprint.0 as u32 {
                    raw_size[0]
                } else {
                    x_start + header.footprint.0 as u32
                };

                write.seek(SeekFrom::Start(next_jackal_block_pos))?;

                // Write a jackal_block.
                let sb = JackalBlock {
                    offset: next_data_pos,
                };
                sb.write_to(&mut write)?;
                next_jackal_block_pos += JackalBlock::BYTES_SIZE as u64;

                write.seek(SeekFrom::Start(next_data_pos))?;
                compress_any_block::<B>(
                    x_start, x_end, y_start, y_end, z, raw_size, blocks, &mut write,
                )?;
                next_data_pos = write.seek(SeekFrom::Current(0))?;
            }
        }
    }

    Ok(())
}

pub fn compress_bc1_blocks(
    header: JackalHeader,
    super_pos: [u32; 3],
    jackal_block: JackalBlock,
    blocks: &[bc1::Block],
    mut write: impl Write + Seek,
) -> std::io::Result<()> {
    let raw_size = header.extent.raw_size();
    let x_start = super_pos[0] * header.footprint.0 as u32;
    let x_end = raw_size[0].min((super_pos[0] + 1) * header.footprint.0 as u32);
    let y_start = super_pos[1] * header.footprint.1 as u32;
    let y_end = raw_size[1].min((super_pos[1] + 1) * header.footprint.1 as u32);
    let z = super_pos[2];

    write.seek(SeekFrom::Start(jackal_block.offset))?;
    compress_any_block(x_start, x_end, y_start, y_end, z, raw_size, blocks, write)
}

fn compress_any_block<B>(
    x_start: u32,
    x_end: u32,
    y_start: u32,
    y_end: u32,
    z: u32,
    raw_size: [u32; 3],
    blocks: &[B],
    mut write: impl Write,
) -> std::io::Result<()>
where
    B: AnyBlock,
{
    let mut encoder = lz78::Encoder::<B::EncoderElement>::new();

    compress_any_block_aspect::<B, 0>(
        x_start,
        x_end,
        y_start,
        y_end,
        z,
        blocks,
        raw_size,
        &mut encoder,
        &mut write,
    )?;

    compress_any_block_aspect::<B, 1>(
        x_start,
        x_end,
        y_start,
        y_end,
        z,
        blocks,
        raw_size,
        &mut encoder,
        &mut write,
    )?;

    compress_any_block_aspect::<B, 2>(
        x_start,
        x_end,
        y_start,
        y_end,
        z,
        blocks,
        raw_size,
        &mut encoder,
        &mut write,
    )?;

    compress_any_block_aspect::<B, 3>(
        x_start,
        x_end,
        y_start,
        y_end,
        z,
        blocks,
        raw_size,
        &mut encoder,
        &mut write,
    )?;

    compress_any_block_aspect::<B, 4>(
        x_start,
        x_end,
        y_start,
        y_end,
        z,
        blocks,
        raw_size,
        &mut encoder,
        &mut write,
    )?;

    compress_any_block_aspect::<B, 5>(
        x_start,
        x_end,
        y_start,
        y_end,
        z,
        blocks,
        raw_size,
        &mut encoder,
        &mut write,
    )?;

    compress_any_block_aspect::<B, 6>(
        x_start,
        x_end,
        y_start,
        y_end,
        z,
        blocks,
        raw_size,
        &mut encoder,
        &mut write,
    )?;

    compress_any_block_aspect::<B, 7>(
        x_start,
        x_end,
        y_start,
        y_end,
        z,
        blocks,
        raw_size,
        &mut encoder,
        &mut write,
    )?;

    encoder.finish(&mut write)?;

    Ok(())
}

fn compress_any_block_aspect<B, const ASPECT: usize>(
    x_start: u32,
    x_end: u32,
    y_start: u32,
    y_end: u32,
    z: u32,
    blocks: &[B],
    raw_size: [u32; 3],
    encoder: &mut lz78::Encoder<B::EncoderElement>,
    write: &mut impl Write,
) -> std::io::Result<()>
where
    B: AnyBlock,
{
    if B::ASPECTS <= ASPECT {
        return Ok(());
    }

    for y in y_start..y_end {
        for x in x_start..x_end {
            let index = x + y * raw_size[0] + z * raw_size[0] * raw_size[1];
            let block = &blocks[index as usize];

            let left = if x > 0 {
                Some(&blocks[index as usize - 1])
            } else {
                None
            };

            let top = if y > 0 {
                Some(&blocks[index as usize - raw_size[0] as usize])
            } else {
                None
            };

            let top_left = if x > 0 && y > 0 {
                Some(&blocks[index as usize - raw_size[0] as usize - 1])
            } else {
                None
            };

            block.compress::<ASPECT>(left, top, top_left, encoder, write)?;
        }
    }

    Ok(())
}

#[derive(Debug)]
pub enum DecompressError {
    Io(std::io::Error),
    Decode(DecodeError),
}

fn lz78_decode_to_decompress_error(err: lz78::DecodeError) -> DecompressError {
    match err {
        lz78::DecodeError::InvalidIndex => DecodeError::InvalidData.into(),
        lz78::DecodeError::Io(err) => DecompressError::Io(err),
    }
}

impl From<std::io::Error> for DecompressError {
    #[inline(always)]
    fn from(err: std::io::Error) -> Self {
        DecompressError::Io(err)
    }
}

impl From<DecodeError> for DecompressError {
    #[inline(always)]
    fn from(err: DecodeError) -> Self {
        DecompressError::Decode(err)
    }
}

/// Read Jackal header from the stream.
pub fn read_header(read: impl Read) -> Result<JackalHeader, DecompressError> {
    JackalHeader::read_from(read)
}

/// Read super-blocks from the stream.
pub fn read_jackal_blocks(
    jackal_blocks: &mut [JackalBlock],
    mut read: impl Read,
) -> Result<(), DecompressError> {
    for sb in jackal_blocks.iter_mut() {
        *sb = JackalBlock::read_from(&mut read)?;
    }
    Ok(())
}

/// Read blocks of one jackal_block.
///
/// `header` is Jackal header.
/// `super_x` is x coordinate of the jackal_block.
/// `super_y` is y coordinate of the jackal_block.
/// `jackal_block` is jackal_block data.
/// `blocks` array of pre-allocated blocks of all jackal_blocks.
pub fn decompress_bc1_blocks(
    header: JackalHeader,
    super_pos: [u32; 3],
    jackal_block: JackalBlock,
    blocks: &mut [bc1::Block],
    read: impl Read + Seek,
) -> Result<(), DecompressError> {
    decompress_any_block(header, super_pos, jackal_block, blocks, read)
}

fn decompress_any_block<B>(
    header: JackalHeader,
    super_pos: [u32; 3],
    jackal_block: JackalBlock,
    blocks: &mut [B],
    mut read: impl Read + Seek,
) -> Result<(), DecompressError>
where
    B: AnyBlock,
{
    let raw_size = header.extent.raw_size();

    let start_x = super_pos[0] * header.footprint.0 as u32;
    let end_x = if super_pos[0] - start_x < header.footprint.0 as u32 {
        raw_size[0]
    } else {
        start_x + header.footprint.0 as u32
    };

    let start_y = super_pos[1] * header.footprint.1 as u32;
    let end_y = if super_pos[1] - start_y < header.footprint.1 as u32 {
        raw_size[1]
    } else {
        start_y + header.footprint.1 as u32
    };

    let z = super_pos[2];

    read.seek(SeekFrom::Start(jackal_block.offset))?;

    let mut decoder = lz78::Decoder::<B::EncoderElement>::new();

    decompress_any_block_aspect::<B, 0>(
        start_x,
        end_x,
        start_y,
        end_y,
        z,
        blocks,
        raw_size,
        &mut decoder,
        &mut read,
    )?;

    decompress_any_block_aspect::<B, 1>(
        start_x,
        end_x,
        start_y,
        end_y,
        z,
        blocks,
        raw_size,
        &mut decoder,
        &mut read,
    )?;

    decompress_any_block_aspect::<B, 2>(
        start_x,
        end_x,
        start_y,
        end_y,
        z,
        blocks,
        raw_size,
        &mut decoder,
        &mut read,
    )?;

    decompress_any_block_aspect::<B, 3>(
        start_x,
        end_x,
        start_y,
        end_y,
        z,
        blocks,
        raw_size,
        &mut decoder,
        &mut read,
    )?;

    decompress_any_block_aspect::<B, 4>(
        start_x,
        end_x,
        start_y,
        end_y,
        z,
        blocks,
        raw_size,
        &mut decoder,
        &mut read,
    )?;

    decompress_any_block_aspect::<B, 5>(
        start_x,
        end_x,
        start_y,
        end_y,
        z,
        blocks,
        raw_size,
        &mut decoder,
        &mut read,
    )?;

    decompress_any_block_aspect::<B, 6>(
        start_x,
        end_x,
        start_y,
        end_y,
        z,
        blocks,
        raw_size,
        &mut decoder,
        &mut read,
    )?;

    decompress_any_block_aspect::<B, 7>(
        start_x,
        end_x,
        start_y,
        end_y,
        z,
        blocks,
        raw_size,
        &mut decoder,
        &mut read,
    )?;

    Ok(())
}

fn decompress_any_block_aspect<B, const ASPECT: usize>(
    x_start: u32,
    x_end: u32,
    y_start: u32,
    y_end: u32,
    z: u32,
    blocks: &mut [B],
    raw_size: [u32; 3],
    decoder: &mut lz78::Decoder<B::EncoderElement>,
    read: &mut impl Read,
) -> Result<(), DecompressError>
where
    B: AnyBlock,
{
    if B::ASPECTS <= ASPECT {
        return Ok(());
    }

    for y in y_start..y_end {
        for x in x_start..x_end {
            let index = x + y * raw_size[0] + z * raw_size[0] * raw_size[1];
            let mut block = blocks[index as usize];

            let left = if x > 0 {
                Some(&blocks[index as usize - 1])
            } else {
                None
            };

            let top = if y > 0 {
                Some(&blocks[index as usize - raw_size[0] as usize])
            } else {
                None
            };

            let top_left = if x > 0 && y > 0 {
                Some(&blocks[index as usize - raw_size[0] as usize - 1])
            } else {
                None
            };

            block.decompress::<ASPECT>(left, top, top_left, decoder, read)?;

            blocks[index as usize] = block;
        }
    }

    Ok(())
}

pub fn decompress_bc1_texture(
    mut read: impl Read + Seek,
) -> Result<(Extent, Vec<bc1::Block>), DecompressError> {
    let header = read_header(&mut read)?;
    let mut jackal_blocks = vec![JackalBlock { offset: 0 }; header.jackal_blocks_count()];
    read_jackal_blocks(&mut jackal_blocks, &mut read)?;

    let mut blocks = vec![bc1::Block::BLACK; header.blocks_count()];

    let jackal_blocks_extent = header.jackal_blocks_extent();

    for z in 0..jackal_blocks_extent[2] {
        for y in 0..jackal_blocks_extent[1] {
            for x in 0..jackal_blocks_extent[0] {
                decompress_bc1_blocks(
                    header,
                    [x, y, z],
                    jackal_blocks[(x
                        + y * jackal_blocks_extent[0]
                        + z * jackal_blocks_extent[0] * jackal_blocks_extent[1])
                        as usize],
                    &mut blocks,
                    &mut read,
                )?;
            }
        }
    }

    Ok((header.extent(), blocks))
}

#[test]
fn roundtrip() {
    use crate::math::Rgb32F;

    let pixels = [
        [Rgb32F::BLACK, Rgb32F::WHITE, Rgb32F::BLACK, Rgb32F::WHITE],
        [Rgb32F::WHITE, Rgb32F::BLACK, Rgb32F::WHITE, Rgb32F::BLACK],
        [Rgb32F::BLACK, Rgb32F::WHITE, Rgb32F::BLACK, Rgb32F::WHITE],
        [Rgb32F::WHITE, Rgb32F::BLACK, Rgb32F::WHITE, Rgb32F::BLACK],
    ];

    let block = bc1::Block::encode(pixels, 1);

    assert_eq!(block.decode(), pixels);

    let blocks = vec![block; 17];

    let mut output = Vec::new();
    compress_bc1_texture(
        Extent::D2 {
            width: 17,
            height: 1,
        },
        &blocks,
        std::io::Cursor::new(&mut output),
    )
    .unwrap();

    let (extent, decompressed) = decompress_bc1_texture(std::io::Cursor::new(&output)).unwrap();

    assert_eq!(
        extent,
        Extent::D2 {
            width: 17,
            height: 1,
        }
    );

    assert_eq!(decompressed[..], blocks[..]);
}