use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};

use super::{Refusal, SIDE_MAX, square_jpeg};

fn encoded(image: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, format)
        .expect("test image encodes");
    bytes.into_inner()
}

fn decoded(jpeg: &[u8]) -> DynamicImage {
    assert_eq!(image::guess_format(jpeg).ok(), Some(ImageFormat::Jpeg));
    image::load_from_memory(jpeg).expect("normalized artwork decodes")
}

#[test]
fn a_wide_png_becomes_its_centered_square() {
    let wide = RgbImage::from_fn(1200, 600, |x, _| {
        if (300..900).contains(&x) {
            Rgb([0, 0, 255])
        } else {
            Rgb([255, 0, 0])
        }
    });

    let square = decoded(&square_jpeg(&encoded(wide.into(), ImageFormat::Png)).unwrap()).to_rgb8();

    assert_eq!(square.dimensions(), (600, 600));
    for (x, y) in [(5, 5), (594, 594), (300, 300)] {
        let [red, _, blue] = square.get_pixel(x, y).0;
        assert!(blue > 200 && red < 60, "pixel {x},{y} is {red}/{blue}");
    }
}

#[test]
fn a_big_photo_is_scaled_down_to_the_side_cap() {
    let photo = RgbImage::from_pixel(2400, 1800, Rgb([40, 120, 200]));

    let square = decoded(&square_jpeg(&encoded(photo.into(), ImageFormat::Jpeg)).unwrap());

    assert_eq!((square.width(), square.height()), (SIDE_MAX, SIDE_MAX));
}

#[test]
fn transparent_pixels_turn_white() {
    let clear = RgbaImage::from_pixel(400, 400, Rgba([0, 0, 0, 0]));

    let square = decoded(&square_jpeg(&encoded(clear.into(), ImageFormat::Png)).unwrap()).to_rgb8();

    assert!(
        square
            .get_pixel(200, 200)
            .0
            .iter()
            .all(|channel| *channel > 245)
    );
}

#[test]
fn a_tiny_image_is_too_small() {
    let tiny = RgbImage::from_pixel(299, 800, Rgb([1, 2, 3]));

    assert_eq!(
        square_jpeg(&encoded(tiny.into(), ImageFormat::Png)),
        Err(Refusal::TooSmall)
    );
}

#[test]
fn a_huge_side_is_too_large() {
    let strip = RgbImage::from_pixel(6001, 400, Rgb([1, 2, 3]));

    assert_eq!(
        square_jpeg(&encoded(strip.into(), ImageFormat::Png)),
        Err(Refusal::TooLarge)
    );
}

#[test]
fn a_gif_or_random_bytes_are_unsupported() {
    assert_eq!(
        square_jpeg(b"GIF89a\x01\x00\x01\x00"),
        Err(Refusal::Unsupported)
    );
    assert_eq!(
        square_jpeg(b"not an image at all"),
        Err(Refusal::Unsupported)
    );
}

#[test]
fn a_jpeg_cut_inside_its_header_is_unreadable() {
    let photo = encoded(
        RgbImage::from_pixel(600, 600, Rgb([9, 9, 9])).into(),
        ImageFormat::Jpeg,
    );

    assert_eq!(square_jpeg(&photo[..64]), Err(Refusal::Unreadable));
}
