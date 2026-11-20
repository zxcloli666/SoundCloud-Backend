use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, Rgb, RgbImage, RgbaImage,
};

pub const SIDE_MAX: u32 = 1000;
pub const SIDE_MIN: u32 = 300;
const INPUT_SIDE_MAX: u32 = 6000;
const DECODE_BYTES_MAX: u64 = 128 * 1024 * 1024;
const QUALITY: u8 = 90;
const FORMATS: [ImageFormat; 3] = [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::WebP];

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Unsupported,
    Unreadable,
    TooLarge,
    TooSmall,
}

pub fn square_jpeg(bytes: &[u8]) -> Result<Vec<u8>, Refusal> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| Refusal::Unreadable)?;
    if !reader
        .format()
        .is_some_and(|format| FORMATS.contains(&format))
    {
        return Err(Refusal::Unsupported);
    }
    let mut decoder = reader.into_decoder().map_err(|_| Refusal::Unreadable)?;
    let (width, height) = decoder.dimensions();
    if width.max(height) > INPUT_SIDE_MAX {
        return Err(Refusal::TooLarge);
    }
    if width.min(height) < SIDE_MIN {
        return Err(Refusal::TooSmall);
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(DECODE_BYTES_MAX);
    decoder.set_limits(limits).map_err(|_| Refusal::TooLarge)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| Refusal::Unreadable)?;
    image.apply_orientation(orientation);
    let side = image.width().min(image.height());
    let square = image.crop_imm(
        (image.width() - side) / 2,
        (image.height() - side) / 2,
        side,
        side,
    );
    let fitted = if side > SIDE_MAX {
        square.resize_exact(SIDE_MAX, SIDE_MAX, FilterType::CatmullRom)
    } else {
        square
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, QUALITY)
        .encode_image(&on_white(&fitted.to_rgba8()))
        .map_err(|_| Refusal::Unreadable)?;
    Ok(jpeg)
}

fn on_white(image: &RgbaImage) -> RgbImage {
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let [red, green, blue, alpha] = image.get_pixel(x, y).0;
        let alpha = u32::from(alpha);
        let blend = |channel: u8| ((u32::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8;
        Rgb([blend(red), blend(green), blend(blue)])
    })
}

#[cfg(test)]
#[path = "normalize_tests.rs"]
mod tests;
