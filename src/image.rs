//! Bounded RGBA image storage and backend-independent CPU effects.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Seek};
use std::num::NonZeroU32;
use std::path::Path;

pub type Rgba = [u8; 4];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageEffect {
    None,
    Blur { radius: u32 },
    Pixelate { block_size: NonZeroU32 },
    BlurPixelate { radius: u32, block_size: NonZeroU32 },
}

impl ImageEffect {
    pub fn apply(self, image: &mut RgbaImage, limits: ImageLimits) -> Result<(), ImageError> {
        match self {
            Self::None => Ok(()),
            Self::Blur { radius } => image.box_blur(radius, limits),
            Self::Pixelate { block_size } => {
                image.pixelate(block_size);
                Ok(())
            }
            Self::BlurPixelate { radius, block_size } => {
                image.box_blur(radius, limits)?;
                image.pixelate(block_size);
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageLimits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_bytes: usize,
    pub max_blur_radius: u32,
    pub max_scratch_bytes: usize,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_width: 16_384,
            max_height: 16_384,
            max_pixels: 64 * 1024 * 1024,
            max_bytes: 256 * 1024 * 1024,
            max_blur_radius: 128,
            max_scratch_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageError {
    ZeroDimension,
    DimensionLimit,
    PixelLimit,
    SizeOverflow,
    ByteLimit,
    InvalidDataLength { expected: usize, actual: usize },
    AllocationFailed,
    BlurRadiusLimit,
    ScratchLimit,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimension => f.write_str("image dimensions must be non-zero"),
            Self::DimensionLimit => f.write_str("image dimensions exceed configured limits"),
            Self::PixelLimit => f.write_str("image pixel count exceeds configured limit"),
            Self::SizeOverflow => f.write_str("image size calculation overflowed"),
            Self::ByteLimit => f.write_str("image byte count exceeds configured limit"),
            Self::InvalidDataLength { expected, actual } => {
                write!(
                    f,
                    "invalid RGBA data length: expected {expected}, got {actual}"
                )
            }
            Self::AllocationFailed => f.write_str("image allocation failed"),
            Self::BlurRadiusLimit => f.write_str("blur radius exceeds configured limit"),
            Self::ScratchLimit => f.write_str("effect scratch space exceeds configured limit"),
        }
    }
}

impl Error for ImageError {}

#[derive(Debug)]
pub enum ImageLoadError {
    Open(io::Error),
    Decode(::image::ImageError),
    Bounds(ImageError),
}

impl fmt::Display for ImageLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(error) => write!(f, "could not open PNG: {error}"),
            Self::Decode(error) => write!(f, "could not decode PNG: {error}"),
            Self::Bounds(error) => write!(f, "decoded PNG is outside safe limits: {error}"),
        }
    }
}

impl Error for ImageLoadError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RgbaImage {
    pub fn solid(
        width: u32,
        height: u32,
        color: Rgba,
        limits: ImageLimits,
    ) -> Result<Self, ImageError> {
        let len = checked_rgba_len(width, height, limits)?;
        let mut pixels = allocate(len)?;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&color);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn from_rgba(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        limits: ImageLimits,
    ) -> Result<Self, ImageError> {
        let expected = checked_rgba_len(width, height, limits)?;
        if pixels.len() != expected {
            return Err(ImageError::InvalidDataLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub(crate) fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// Average each complete or partial block independently.
    pub fn pixelate(&mut self, block_size: NonZeroU32) {
        let block = block_size.get() as usize;
        let width = self.width as usize;
        let height = self.height as usize;

        for top in (0..height).step_by(block) {
            let bottom = top.saturating_add(block).min(height);
            for left in (0..width).step_by(block) {
                let right = left.saturating_add(block).min(width);
                let mut sums = [0_u64; 4];
                let count = (bottom - top) as u64 * (right - left) as u64;

                for y in top..bottom {
                    for x in left..right {
                        let offset = (y * width + x) * 4;
                        for (channel, sum) in sums.iter_mut().enumerate() {
                            *sum += u64::from(self.pixels[offset + channel]);
                        }
                    }
                }

                let average = sums.map(|sum| (sum / count) as u8);
                for y in top..bottom {
                    for x in left..right {
                        let offset = (y * width + x) * 4;
                        self.pixels[offset..offset + 4].copy_from_slice(&average);
                    }
                }
            }
        }
    }

    /// Apply a separable box blur. Edge windows contain only real pixels;
    /// there is no implicit transparent or repeated border.
    pub fn box_blur(&mut self, radius: u32, limits: ImageLimits) -> Result<(), ImageError> {
        if radius == 0 {
            return Ok(());
        }
        if radius > limits.max_blur_radius {
            return Err(ImageError::BlurRadiusLimit);
        }

        let scratch = self
            .pixels
            .len()
            .checked_mul(2)
            .ok_or(ImageError::SizeOverflow)?;
        if scratch > limits.max_scratch_bytes {
            return Err(ImageError::ScratchLimit);
        }

        let mut horizontal = allocate(self.pixels.len())?;
        let mut output = allocate(self.pixels.len())?;
        blur_horizontal(
            &self.pixels,
            &mut horizontal,
            self.width as usize,
            self.height as usize,
            radius as usize,
        );
        blur_vertical(
            &horizontal,
            &mut output,
            self.width as usize,
            self.height as usize,
            radius as usize,
        );
        self.pixels = output;
        Ok(())
    }

    pub fn fitted(
        &self,
        width: u32,
        height: u32,
        mode: FitMode,
        background: Rgba,
        limits: ImageLimits,
    ) -> Result<Self, ImageError> {
        let mut output = Self::solid(width, height, background, limits)?;
        match mode {
            FitMode::Center => blit_center(self, &mut output),
            FitMode::Contain => blit_scaled(self, &mut output, ScaleMode::Contain),
            FitMode::Cover => blit_scaled(self, &mut output, ScaleMode::Cover),
        }
        Ok(output)
    }
}

/// Decode exactly one PNG under the same dimensions and memory ceiling used
/// by the internal RGBA representation.
pub fn load_png(path: &Path, limits: ImageLimits) -> Result<RgbaImage, ImageLoadError> {
    let file = File::open(path).map_err(ImageLoadError::Open)?;
    decode_png(BufReader::new(file), limits)
}

/// Decode a PNG already resident in memory under the normal image limits.
pub fn load_png_bytes(bytes: &[u8], limits: ImageLimits) -> Result<RgbaImage, ImageLoadError> {
    decode_png(Cursor::new(bytes), limits)
}

fn decode_png<R: BufRead + Seek>(
    input: R,
    limits: ImageLimits,
) -> Result<RgbaImage, ImageLoadError> {
    let mut reader = ::image::ImageReader::with_format(input, ::image::ImageFormat::Png);
    let mut decode_limits = ::image::Limits::default();
    decode_limits.max_image_width = Some(limits.max_width);
    decode_limits.max_image_height = Some(limits.max_height);
    decode_limits.max_alloc = Some(limits.max_bytes as u64);
    reader.limits(decode_limits);
    let decoded = reader.decode().map_err(ImageLoadError::Decode)?;
    let rgba = decoded.into_rgba8();
    RgbaImage::from_rgba(rgba.width(), rgba.height(), rgba.into_raw(), limits)
        .map_err(ImageLoadError::Bounds)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FitMode {
    Cover,
    Contain,
    Center,
}

#[derive(Clone, Copy)]
enum ScaleMode {
    Cover,
    Contain,
}

fn checked_rgba_len(width: u32, height: u32, limits: ImageLimits) -> Result<usize, ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::ZeroDimension);
    }
    if width > limits.max_width || height > limits.max_height {
        return Err(ImageError::DimensionLimit);
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ImageError::SizeOverflow)?;
    if pixels > limits.max_pixels {
        return Err(ImageError::PixelLimit);
    }
    let bytes = pixels.checked_mul(4).ok_or(ImageError::SizeOverflow)?;
    let bytes = usize::try_from(bytes).map_err(|_| ImageError::SizeOverflow)?;
    if bytes > limits.max_bytes {
        return Err(ImageError::ByteLimit);
    }
    Ok(bytes)
}

fn allocate(len: usize) -> Result<Vec<u8>, ImageError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| ImageError::AllocationFailed)?;
    bytes.resize(len, 0);
    Ok(bytes)
}

fn blur_horizontal(source: &[u8], target: &mut [u8], width: usize, height: usize, radius: usize) {
    for y in 0..height {
        let mut left = 0;
        let mut right = radius.saturating_add(1).min(width);
        let mut sums = [0_u64; 4];
        for x in left..right {
            add_pixel(&mut sums, source, (y * width + x) * 4);
        }

        for x in 0..width {
            write_average(target, (y * width + x) * 4, sums, right - left);
            if x + 1 == width {
                break;
            }
            let new_left = (x + 1).saturating_sub(radius);
            while left < new_left {
                subtract_pixel(&mut sums, source, (y * width + left) * 4);
                left += 1;
            }
            let new_right = (x + 1).saturating_add(radius).saturating_add(1).min(width);
            while right < new_right {
                add_pixel(&mut sums, source, (y * width + right) * 4);
                right += 1;
            }
        }
    }
}

fn blur_vertical(source: &[u8], target: &mut [u8], width: usize, height: usize, radius: usize) {
    for x in 0..width {
        let mut top = 0;
        let mut bottom = radius.saturating_add(1).min(height);
        let mut sums = [0_u64; 4];
        for y in top..bottom {
            add_pixel(&mut sums, source, (y * width + x) * 4);
        }

        for y in 0..height {
            write_average(target, (y * width + x) * 4, sums, bottom - top);
            if y + 1 == height {
                break;
            }
            let new_top = (y + 1).saturating_sub(radius);
            while top < new_top {
                subtract_pixel(&mut sums, source, (top * width + x) * 4);
                top += 1;
            }
            let new_bottom = (y + 1).saturating_add(radius).saturating_add(1).min(height);
            while bottom < new_bottom {
                add_pixel(&mut sums, source, (bottom * width + x) * 4);
                bottom += 1;
            }
        }
    }
}

fn add_pixel(sums: &mut [u64; 4], pixels: &[u8], offset: usize) {
    for channel in 0..4 {
        sums[channel] += u64::from(pixels[offset + channel]);
    }
}

fn subtract_pixel(sums: &mut [u64; 4], pixels: &[u8], offset: usize) {
    for channel in 0..4 {
        sums[channel] -= u64::from(pixels[offset + channel]);
    }
}

fn write_average(target: &mut [u8], offset: usize, sums: [u64; 4], count: usize) {
    for channel in 0..4 {
        target[offset + channel] = (sums[channel] / count as u64) as u8;
    }
}

fn blit_center(source: &RgbaImage, target: &mut RgbaImage) {
    let copy_width = source.width.min(target.width) as usize;
    let copy_height = source.height.min(target.height) as usize;
    let source_x = (source.width as usize - copy_width) / 2;
    let source_y = (source.height as usize - copy_height) / 2;
    let target_x = (target.width as usize - copy_width) / 2;
    let target_y = (target.height as usize - copy_height) / 2;

    for row in 0..copy_height {
        let source_start = ((source_y + row) * source.width as usize + source_x) * 4;
        let target_start = ((target_y + row) * target.width as usize + target_x) * 4;
        let bytes = copy_width * 4;
        target.pixels[target_start..target_start + bytes]
            .copy_from_slice(&source.pixels[source_start..source_start + bytes]);
    }
}

fn blit_scaled(source: &RgbaImage, target: &mut RgbaImage, mode: ScaleMode) {
    let sw = u64::from(source.width);
    let sh = u64::from(source.height);
    let tw = u64::from(target.width);
    let th = u64::from(target.height);
    let target_is_wider = tw * sh >= th * sw;

    let (scaled_width, scaled_height) = match (mode, target_is_wider) {
        (ScaleMode::Cover, true) | (ScaleMode::Contain, false) => (tw, (sh * tw / sw).max(1)),
        (ScaleMode::Cover, false) | (ScaleMode::Contain, true) => ((sw * th / sh).max(1), th),
    };

    match mode {
        ScaleMode::Contain => {
            let origin_x = (tw - scaled_width) / 2;
            let origin_y = (th - scaled_height) / 2;
            for y in 0..scaled_height {
                for x in 0..scaled_width {
                    copy_scaled_pixel(
                        source,
                        target,
                        x * sw / scaled_width,
                        y * sh / scaled_height,
                        origin_x + x,
                        origin_y + y,
                    );
                }
            }
        }
        ScaleMode::Cover => {
            let crop_x = (scaled_width - tw) / 2;
            let crop_y = (scaled_height - th) / 2;
            for y in 0..th {
                for x in 0..tw {
                    copy_scaled_pixel(
                        source,
                        target,
                        (crop_x + x) * sw / scaled_width,
                        (crop_y + y) * sh / scaled_height,
                        x,
                        y,
                    );
                }
            }
        }
    }
}

fn copy_scaled_pixel(
    source: &RgbaImage,
    target: &mut RgbaImage,
    source_x: u64,
    source_y: u64,
    target_x: u64,
    target_y: u64,
) {
    let source_offset = ((source_y * u64::from(source.width) + source_x) * 4) as usize;
    let target_offset = ((target_y * u64::from(target.width) + target_x) * 4) as usize;
    target.pixels[target_offset..target_offset + 4]
        .copy_from_slice(&source.pixels[source_offset..source_offset + 4]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
    use std::io::Cursor;

    const LIMITS: ImageLimits = ImageLimits {
        max_width: 32,
        max_height: 32,
        max_pixels: 1024,
        max_bytes: 4096,
        max_blur_radius: 8,
        max_scratch_bytes: 8192,
    };

    #[test]
    fn rejects_zero_and_oversized_images_before_allocation() {
        assert_eq!(
            RgbaImage::solid(0, 1, [0; 4], LIMITS),
            Err(ImageError::ZeroDimension)
        );
        assert_eq!(
            RgbaImage::solid(33, 1, [0; 4], LIMITS),
            Err(ImageError::DimensionLimit)
        );
    }

    #[test]
    fn validates_rgba_buffer_length() {
        assert_eq!(
            RgbaImage::from_rgba(2, 2, vec![0; 15], LIMITS),
            Err(ImageError::InvalidDataLength {
                expected: 16,
                actual: 15
            })
        );
    }

    #[test]
    fn pixelation_uses_actual_partial_block_size() {
        let mut image = RgbaImage::from_rgba(
            3,
            2,
            vec![
                0, 0, 0, 255, 10, 10, 10, 255, 100, 100, 100, 255, 20, 20, 20, 255, 30, 30, 30,
                255, 200, 200, 200, 255,
            ],
            LIMITS,
        )
        .unwrap();

        image.pixelate(NonZeroU32::new(2).unwrap());

        assert_eq!(
            image.pixels(),
            &[
                15, 15, 15, 255, 15, 15, 15, 255, 150, 150, 150, 255, 15, 15, 15, 255, 15, 15, 15,
                255, 150, 150, 150, 255,
            ]
        );
    }

    #[test]
    fn blur_radius_zero_is_an_identity_without_allocation() {
        let original = RgbaImage::solid(2, 2, [1, 2, 3, 255], LIMITS).unwrap();
        let mut image = original.clone();
        image.box_blur(0, LIMITS).unwrap();
        assert_eq!(image, original);
    }

    #[test]
    fn blur_handles_edges_using_only_real_pixels() {
        let mut image = RgbaImage::from_rgba(
            3,
            1,
            vec![0, 0, 0, 255, 90, 90, 90, 255, 210, 210, 210, 255],
            LIMITS,
        )
        .unwrap();
        image.box_blur(1, LIMITS).unwrap();

        assert_eq!(
            image.pixels(),
            &[45, 45, 45, 255, 100, 100, 100, 255, 150, 150, 150, 255]
        );
    }

    #[test]
    fn blur_enforces_radius_and_scratch_limits() {
        let mut image = RgbaImage::solid(2, 2, [0; 4], LIMITS).unwrap();
        assert_eq!(image.box_blur(9, LIMITS), Err(ImageError::BlurRadiusLimit));

        let mut tight = LIMITS;
        tight.max_scratch_bytes = 31;
        assert_eq!(image.box_blur(1, tight), Err(ImageError::ScratchLimit));
    }

    #[test]
    fn combined_effect_blurs_before_pixelating() {
        let source = RgbaImage::from_rgba(
            3,
            1,
            vec![0, 0, 0, 255, 90, 90, 90, 255, 210, 210, 210, 255],
            LIMITS,
        )
        .unwrap();
        let mut expected = source.clone();
        expected.box_blur(1, LIMITS).unwrap();
        expected.pixelate(NonZeroU32::new(2).unwrap());

        let mut actual = source;
        ImageEffect::BlurPixelate {
            radius: 1,
            block_size: NonZeroU32::new(2).unwrap(),
        }
        .apply(&mut actual, LIMITS)
        .unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn contain_keeps_background_bars_opaque() {
        let source = RgbaImage::solid(2, 1, [10, 20, 30, 255], LIMITS).unwrap();
        let output = source
            .fitted(2, 2, FitMode::Contain, [1, 2, 3, 255], LIMITS)
            .unwrap();

        assert_eq!(
            output.pixels(),
            &[10, 20, 30, 255, 10, 20, 30, 255, 1, 2, 3, 255, 1, 2, 3, 255,]
        );
    }

    #[test]
    fn center_crops_source_symmetrically() {
        let source =
            RgbaImage::from_rgba(3, 1, vec![1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255], LIMITS)
                .unwrap();
        let output = source
            .fitted(1, 1, FitMode::Center, [0; 4], LIMITS)
            .unwrap();
        assert_eq!(output.pixels(), &[2, 0, 0, 255]);
    }

    #[test]
    fn cover_fills_every_pixel() {
        let source = RgbaImage::solid(1, 2, [7, 8, 9, 255], LIMITS).unwrap();
        let output = source
            .fitted(3, 1, FitMode::Cover, [0, 0, 0, 0], LIMITS)
            .unwrap();
        assert!(
            output
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [7, 8, 9, 255])
        );
    }

    #[test]
    fn png_decoder_accepts_rgba_under_limits() {
        let pixels = [1, 2, 3, 255, 4, 5, 6, 128];
        let mut encoded = Vec::new();
        PngEncoder::new(&mut encoded)
            .write_image(&pixels, 2, 1, ExtendedColorType::Rgba8)
            .unwrap();

        let decoded = decode_png(Cursor::new(encoded), LIMITS).unwrap();
        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 1);
        assert_eq!(decoded.pixels(), pixels);
    }

    #[test]
    fn png_decoder_rejects_dimensions_before_output_allocation() {
        let pixels = vec![0_u8; 33 * 4];
        let mut encoded = Vec::new();
        PngEncoder::new(&mut encoded)
            .write_image(&pixels, 33, 1, ExtendedColorType::Rgba8)
            .unwrap();

        let result = decode_png(Cursor::new(encoded), LIMITS);
        assert!(matches!(result, Err(ImageLoadError::Decode(_))));
    }
}
