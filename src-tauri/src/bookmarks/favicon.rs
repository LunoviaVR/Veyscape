//! Site icons for bookmarks. Icons come only from pages the user loads (or from an imported
//! bookmarks file), never from extra network requests. They are stored as small PNG data URLs:
//! anything larger than [`STORED_SIZE`] is scaled down first.

use base64::Engine;

const PREFIX: &str = "data:image/png;base64,";
/// Largest edge kept in the database: crisp at 16px on 2× displays.
pub const STORED_SIZE: u32 = 32;
/// Decoding stops beyond this many bytes of pixels (guards against decompression bombs).
const DECODE_LIMIT: usize = 8 * 1024 * 1024;

/// An image as straight (non-premultiplied) RGBA rows.
pub struct Rgba {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A PNG data URL ready to store, or None if `data_url` is not a decodable PNG.
pub fn normalize(data_url: &str) -> Option<String> {
    // Always re-encoded, even when small: the source may carry megabytes of ancillary chunks.
    let image = rgba(data_url, STORED_SIZE)?;
    Some(format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(encode(&image)?)
    ))
}

/// The icon at no more than `size` pixels square (for native menus).
pub fn rgba(data_url: &str, size: u32) -> Option<Rgba> {
    let png = data_url
        .strip_prefix(PREFIX)
        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())?;
    let image = decode(&png)?;
    Some(if image.width > size || image.height > size {
        scale_to_fit(&image, size)
    } else {
        image
    })
}

fn decode(png: &[u8]) -> Option<Rgba> {
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(png),
        png::Limits {
            bytes: DECODE_LIMIT,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    // `Limits` does not cover our output buffer, whose size comes straight from the header.
    let size = reader.output_buffer_size().filter(|&n| n <= DECODE_LIMIT)?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        // Palette images are expanded by `normalize_to_color8`.
        png::ColorType::Indexed => return None,
    };
    (info.width > 0 && info.height > 0).then_some(Rgba {
        pixels,
        width: info.width,
        height: info.height,
    })
}

fn encode(image: &Rgba) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&image.pixels).ok()?;
    writer.finish().ok()?;
    Some(out)
}

/// Box-filter downscale so the longer edge is `size`, averaging colour weighted by alpha so
/// transparent edges do not darken.
fn scale_to_fit(src: &Rgba, size: u32) -> Rgba {
    let longest = src.width.max(src.height);
    let (w, h) = (
        (src.width * size / longest).max(1),
        (src.height * size / longest).max(1),
    );
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let (y0, y1) = span(y, h, src.height);
        for x in 0..w {
            let (x0, x1) = span(x, w, src.width);
            let (mut rgb, mut alpha, mut count) = ([0u64; 3], 0u64, 0u64);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * src.width + sx) * 4) as usize;
                    let a = u64::from(src.pixels[i + 3]);
                    for (sum, &v) in rgb.iter_mut().zip(&src.pixels[i..i + 3]) {
                        *sum += u64::from(v) * a;
                    }
                    alpha += a;
                    count += 1;
                }
            }
            let channel = |c: usize| rgb[c].checked_div(alpha).unwrap_or(0) as u8;
            pixels.extend([channel(0), channel(1), channel(2), (alpha / count) as u8]);
        }
    }
    Rgba {
        pixels,
        width: w,
        height: h,
    }
}

/// Source pixel range covered by output pixel `i` of `out` (at least one pixel).
fn span(i: u32, out: u32, src: u32) -> (u32, u32) {
    let start = i * src / out;
    (start, ((i + 1) * src / out).max(start + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_url(image: &Rgba) -> String {
        format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(encode(image).unwrap())
        )
    }

    fn solid(size: u32, px: [u8; 4]) -> Rgba {
        Rgba {
            pixels: px.repeat((size * size) as usize),
            width: size,
            height: size,
        }
    }

    #[test]
    fn large_icons_are_scaled_down_and_keep_their_colour() {
        let big = data_url(&solid(128, [200, 30, 40, 255]));
        let stored = normalize(&big).unwrap();
        let back = rgba(&stored, 64).unwrap();
        assert_eq!((back.width, back.height), (STORED_SIZE, STORED_SIZE));
        assert_eq!(&back.pixels[..4], &[200, 30, 40, 255]);
        assert!(stored.len() < big.len());
    }

    #[test]
    fn small_icons_are_kept_as_they_are() {
        let small = data_url(&solid(16, [1, 2, 3, 255]));
        assert_eq!(normalize(&small).as_deref(), Some(small.as_str()));
    }

    /// A PNG of the given size whose header is followed by `chunk` and then `pixels` (if any).
    fn png_with(
        width: u32,
        height: u32,
        chunk: png::chunk::ChunkType,
        data: &[u8],
        pixels: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer.write_chunk(chunk, data).unwrap();
        if let Some(p) = pixels {
            writer.write_image_data(p).unwrap();
        }
        writer.finish().unwrap();
        out
    }

    #[test]
    fn header_claiming_a_huge_image_is_refused_before_allocating() {
        // 65535 × (2^31 - 1) RGBA: hundreds of terabytes promised by a 57-byte file.
        let bomb = png_with(65535, i32::MAX as u32, png::chunk::IDAT, &[], None);
        assert!(bomb.len() < 100);
        assert!(decode(&bomb).is_none());
    }

    #[test]
    fn small_icons_drop_extra_chunks() {
        let padded = png_with(
            1,
            1,
            png::chunk::ChunkType(*b"zzZz"),
            &vec![7; 1 << 20],
            Some(&[1, 2, 3, 255]),
        );
        let url = format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(&padded)
        );
        let stored = normalize(&url).unwrap();
        assert!(stored.len() < 200, "{} bytes stored", stored.len());
        assert_eq!(rgba(&stored, 32).unwrap().pixels, [1, 2, 3, 255]);
    }

    #[test]
    fn transparent_pixels_do_not_darken_the_average() {
        let mut half = solid(2, [255, 255, 255, 255]);
        half.pixels[4..8].copy_from_slice(&[0, 0, 0, 0]);
        let one = scale_to_fit(&half, 1);
        assert_eq!(&one.pixels[..3], &[255, 255, 255]);
    }

    #[test]
    fn non_png_and_broken_data_are_rejected() {
        assert!(normalize("data:image/svg+xml;base64,PHN2Zy8+").is_none());
        assert!(normalize("data:image/png;base64,not-base64!").is_none());
        assert!(normalize("data:image/png;base64,AAAA").is_none());
        assert!(normalize("https://example.com/favicon.ico").is_none());
    }
}
