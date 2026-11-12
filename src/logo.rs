//! The restaurant's logo at the top of a receipt. It is downloaded once and kept in `%APPDATA%\restora\logos`, so a
//! receipt printed without internet still has it; then it is printed as an ESC/POS raster image (`GS v 0`).

use crate::config;
use base64::Engine;
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use std::{io::Cursor, net::IpAddr, path::PathBuf, sync::OnceLock};

/// The longest a logo download may take; a receipt is not held up longer for its logo.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// The largest logo file kept.
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_PIXELS: u64 = 4_000_000;

/// Where a logo is kept: one file per address, so a new logo (a new address) is downloaded again.
fn cached(url: &str) -> PathBuf {
    // FNV-1a: a short, stable file name for the address.
    let hash = url.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    let folder = config::folder().join("logos");
    let _ = std::fs::create_dir_all(&folder);
    folder.join(format!("{hash:016x}"))
}

/// Logos may be external, but must never be allowed to fetch loopback/private
/// network resources through the Windows agent. No credentials or arbitrary ports.
fn safe_remote_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if url.port().is_some_and(|port| port != 443) {
        return false;
    }
    url.host_str().is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        host != "localhost"
            && !host.ends_with(".localhost")
            && !host.ends_with(".local")
            && host.parse::<IpAddr>().is_err()
    })
}

fn safe_image(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_BYTES {
        return false;
    }
    let reader = image::ImageReader::new(Cursor::new(bytes));
    let Ok(reader) = reader.with_guessed_format() else {
        return false;
    };
    let Ok((width, height)) = reader.into_dimensions() else {
        return false;
    };
    width > 0 && height > 0 && (width as u64) * (height as u64) <= MAX_PIXELS
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() > 3 || !safe_remote_url(attempt.url()) {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .expect("HTTPS logo client")
    })
}

/// Cached first, otherwise streamed with a strict byte limit. Only HTTPS remote
/// images (with safe redirects) and bounded image data URLs are accepted.
pub async fn load(url: Option<&str>) -> Option<Vec<u8>> {
    let url = url.filter(|value| !value.is_empty())?;
    if let Some(data) = url.strip_prefix("data:image/") {
        let (_, encoded) = data.split_once(";base64,")?;
        if encoded.len() > (MAX_BYTES * 4 / 3 + 8) {
            return None;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        return safe_image(&bytes).then_some(bytes);
    }

    let remote = reqwest::Url::parse(url).ok()?;
    if !safe_remote_url(&remote) {
        return None;
    }
    let path = cached(url);
    if let Ok(bytes) = std::fs::read(&path) {
        if safe_image(&bytes) {
            return Some(bytes);
        }
    }

    let mut response = http()
        .get(remote)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    if !safe_remote_url(response.url()) {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    if !safe_image(&bytes) {
        return None;
    }
    if let Err(error) = std::fs::write(&path, &bytes) {
        tracing::warn!("cannot cache restaurant receipt logo: {error}");
    }
    Some(bytes)
}

/// The ESC/POS bytes that print `file` (PNG, JPEG, WebP or GIF) centred on a roll `width` characters wide: at most half
/// the roll wide and 160 dots tall, in black and white with Floyd–Steinberg dithering, see-through parts white.
pub fn raster(file: &[u8], width: usize) -> Option<Vec<u8>> {
    if !safe_image(file) {
        return None;
    }
    let image = image::load_from_memory(file).ok()?;
    // 12 dots per character: 576 dots on an 80 mm roll, 384 on 58 mm.
    let (max_w, max_h) = ((width * 12 / 2) as u32, 160u32);
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let scale = (max_w as f32 / w as f32)
        .min(max_h as f32 / h as f32)
        .min(1.0);
    let (w, h) = (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    );
    let image = DynamicImage::ImageRgba8(image.resize_exact(w, h, FilterType::Lanczos3).to_rgba8());

    // Grey on white: see-through pixels count as paper.
    let mut grey: Vec<f32> = image
        .to_rgba8()
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0.map(|v| v as f32);
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            luma * a / 255.0 + 255.0 * (1.0 - a / 255.0)
        })
        .collect();

    let (w, h) = (w as usize, h as usize);
    let row_bytes = w.div_ceil(8);
    let mut bits = vec![0u8; row_bytes * h];
    for y in 0..h {
        for x in 0..w {
            let old = grey[y * w + x];
            let black = old < 128.0;
            if black {
                bits[y * row_bytes + x / 8] |= 0x80 >> (x % 8);
            }
            let error = old - if black { 0.0 } else { 255.0 };
            let mut spread = |dx: isize, dy: usize, share: f32| {
                let nx = x as isize + dx;
                if nx >= 0 && (nx as usize) < w && y + dy < h {
                    grey[(y + dy) * w + nx as usize] += error * share;
                }
            };
            spread(1, 0, 7.0 / 16.0);
            spread(-1, 1, 3.0 / 16.0);
            spread(0, 1, 5.0 / 16.0);
            spread(1, 1, 1.0 / 16.0);
        }
    }

    // GS v 0: raster bit image, normal size; width in bytes and height in dots, low byte first.
    let mut bytes = vec![
        0x1d,
        b'v',
        b'0',
        0,
        (row_bytes & 0xff) as u8,
        (row_bytes >> 8) as u8,
        (h & 0xff) as u8,
        (h >> 8) as u8,
    ];
    bytes.extend(bits);
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};

    fn png(w: u32, h: u32, pixel: impl Fn(u32, u32) -> Rgba<u8>) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        RgbaImage::from_fn(w, h, pixel)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_wide_logo_fits_half_the_roll() {
        let bytes = raster(&png(1000, 100, |_, _| Rgba([0, 0, 0, 255])), 48).unwrap();
        assert_eq!(&bytes[..4], &[0x1d, b'v', b'0', 0]);
        let row_bytes = bytes[4] as usize + ((bytes[5] as usize) << 8);
        let height = bytes[6] as usize + ((bytes[7] as usize) << 8);
        assert_eq!(row_bytes, 288 / 8);
        assert_eq!(height, 29);
        assert_eq!(bytes.len(), 8 + row_bytes * height);
        assert!(
            bytes[8..].iter().all(|b| *b == 0xff),
            "solid black prints solid"
        );
    }

    #[test]
    fn a_tall_logo_is_at_most_160_dots_and_see_through_is_white() {
        let bytes = raster(&png(200, 800, |_, _| Rgba([0, 0, 0, 0])), 32).unwrap();
        assert_eq!(bytes[6] as usize + ((bytes[7] as usize) << 8), 160);
        assert!(
            bytes[8..].iter().all(|b| *b == 0),
            "see-through prints nothing"
        );
    }

    #[test]
    fn mid_grey_is_dithered_about_half_black() {
        let bytes = raster(&png(160, 160, |_, _| Rgba([128, 128, 128, 255])), 48).unwrap();
        let black: u32 = bytes[8..].iter().map(|b| b.count_ones()).sum();
        let share = black as f32 / (160.0 * 160.0);
        assert!((0.4..0.6).contains(&share), "{share}");
    }

    #[test]
    fn not_an_image_prints_nothing() {
        assert!(raster(b"<html>", 48).is_none());
    }

    #[test]
    fn receipt_logos_reject_private_endpoints_and_decompression_bombs() {
        for bad in [
            "http://example.org/logo.png",
            "https://127.0.0.1/logo.png",
            "https://192.168.1.2/logo.png",
            "https://localhost/logo.png",
            "https://printer.local/logo.png",
            "https://user:pass@example.org/logo.png",
        ] {
            assert!(
                !safe_remote_url(&reqwest::Url::parse(bad).unwrap()),
                "{bad}"
            );
        }
        assert!(safe_remote_url(
            &reqwest::Url::parse("https://storage.googleapis.com/logo.png").unwrap()
        ));
        assert!(!safe_image(&png(3000, 3000, |_, _| Rgba([0, 0, 0, 255]))));
    }

    #[tokio::test]
    async fn a_data_address_needs_no_internet() {
        let file = png(4, 4, |_, _| Rgba([0, 0, 0, 255]));
        let url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&file)
        );
        assert_eq!(load(Some(&url)).await, Some(file));
        assert_eq!(load(None).await, None);
    }
}
