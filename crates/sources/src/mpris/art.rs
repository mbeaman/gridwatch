//! Album art (brief arc 6 seam 2): fetch a `file://`, `https://` or `data:`
//! URL, decode it with `image` 0.25, downscale so the long side is at most
//! `art_max_px`, and hand back the RGB8 Record the halfblock painter draws.
//! Runs on a blocking task, never in the select loop and never on the render
//! thread.

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use gridwatch_store::keys::media::Art;

/// The most bytes fetched for one cover (the brief's cap).
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// What the decoder will accept before the downscale: a cover, not a
/// poster.
pub const MAX_DECODE_PX: u32 = 8192;
pub const MAX_DECODE_BYTES: u64 = 64 << 20;

#[derive(Debug)]
pub enum ArtError {
    Unsupported(String),
    Io(String),
    TooBig(usize),
    Decode(String),
}

impl std::fmt::Display for ArtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtError::Unsupported(s) => write!(f, "unsupported art URL: {s}"),
            ArtError::Io(s) => write!(f, "art fetch: {s}"),
            ArtError::TooBig(n) => write!(f, "art is {n} bytes, over the 8 MB cap"),
            ArtError::Decode(s) => write!(f, "art decode: {s}"),
        }
    }
}

/// `file:///path` → a path, with the usual percent-decoding for spaces.
pub fn file_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // `file:///x` → `/x`; a host part is not something a player sends.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    Some(PathBuf::from(percent_decode(path)))
}

/// Minimal percent-decoding (`%20` and friends): art URLs are file paths,
/// not a reason to add a dependency.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `data:image/png;base64,…` → the bytes.
pub fn data_url(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if meta.ends_with(";base64") {
        base64_decode(payload)
    } else {
        Some(percent_decode(payload).into_bytes())
    }
}

/// Standard base64 (RFC 4648) — twenty lines rather than a dependency.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Fetch the bytes behind an art URL.
pub fn fetch(url: &str) -> Result<Vec<u8>, ArtError> {
    if let Some(bytes) = data_url(url) {
        return Ok(bytes);
    }
    if let Some(path) = file_path(url) {
        let meta = std::fs::metadata(&path).map_err(|e| ArtError::Io(e.to_string()))?;
        if meta.len() as usize > MAX_BYTES {
            return Err(ArtError::TooBig(meta.len() as usize));
        }
        return std::fs::read(&path).map_err(|e| ArtError::Io(e.to_string()));
    }
    if url.starts_with("https://") {
        // https only, and no downgrade through a redirect: a cover is not
        // worth a plaintext request (review).
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .max_redirects(3)
            .https_only(true)
            .build()
            .new_agent();
        let mut resp = agent
            .get(url)
            .call()
            .map_err(|e| ArtError::Io(e.to_string()))?;
        let mut buf = Vec::new();
        resp.body_mut()
            .as_reader()
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut buf)
            .map_err(|e| ArtError::Io(e.to_string()))?;
        if buf.len() > MAX_BYTES {
            return Err(ArtError::TooBig(buf.len()));
        }
        return Ok(buf);
    }
    Err(ArtError::Unsupported(url.to_string()))
}

/// Decode and downscale to the Record the store carries.
pub fn decode(bytes: &[u8], track: u64, max_px: u32) -> Result<Art, ArtError> {
    // A 1 MB PNG can declare 12000x12000 and decode to 430 MB; the decoder
    // is told what this dashboard will accept (review).
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_PX);
    limits.max_image_height = Some(MAX_DECODE_PX);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ArtError::Decode(e.to_string()))?;
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|e| ArtError::Decode(e.to_string()))?;
    let max_px = max_px.clamp(16, 512);
    let img = if img.width().max(img.height()) > max_px {
        img.thumbnail(max_px, max_px)
    } else {
        img
    };
    let rgb = img.to_rgb8();
    Ok(Art {
        track,
        w: rgb.width().min(u32::from(u16::MAX)) as u16,
        h: rgb.height().min(u32::from(u16::MAX)) as u16,
        rgb: rgb.into_raw(),
    })
}

/// Fetch and decode in one go (the blocking task's body).
pub fn load(url: &str, track: u64, max_px: u32) -> Result<Art, ArtError> {
    let bytes = fetch(url)?;
    decode(&bytes, track, max_px)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut img = image::RgbImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([(x * 4) as u8, (y * 4) as u8, 128]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn decodes_and_downscales() {
        let art = decode(&png(400, 200), 7, 256).unwrap();
        assert_eq!(art.track, 7);
        assert!(art.is_valid());
        assert_eq!(art.w, 256, "the long side is capped");
        assert_eq!(art.h, 128, "the aspect ratio survives");
        // A small cover is left alone.
        let art = decode(&png(64, 64), 1, 256).unwrap();
        assert_eq!((art.w, art.h), (64, 64));
        assert!(matches!(
            decode(b"not a png", 1, 256),
            Err(ArtError::Decode(_))
        ));
    }

    #[test]
    fn file_data_and_bad_urls() {
        let dir = std::env::temp_dir().join(format!("gw-art-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cover art.png");
        std::fs::write(&path, png(32, 32)).unwrap();
        let url = format!("file://{}", path.display().to_string().replace(' ', "%20"));
        let art = load(&url, 3, 256).unwrap();
        assert_eq!((art.w, art.h), (32, 32));
        assert_eq!(art.track, 3);
        assert_eq!(
            file_path("file:///home/a%20b/c.png").unwrap(),
            PathBuf::from("/home/a b/c.png")
        );
        assert!(file_path("https://x/y.png").is_none());
        // data: URLs, base64 and plain.
        let b64 = format!(
            "data:image/png;base64,{}",
            base64_encode_for_test(&png(8, 8))
        );
        let art = decode(&data_url(&b64).unwrap(), 4, 256).unwrap();
        assert_eq!((art.w, art.h), (8, 8));
        assert_eq!(data_url("data:text/plain,hi").unwrap(), b"hi");
        assert!(data_url("https://x").is_none());
        assert!(matches!(
            load("ftp://x/y.png", 1, 256),
            Err(ArtError::Unsupported(_))
        ));
        assert!(
            matches!(
                load("http://x/y.png", 1, 256),
                Err(ArtError::Unsupported(_))
            ),
            "plain http is not fetched"
        );
        assert!(matches!(
            load("file:///nonexistent/x.png", 1, 256),
            Err(ArtError::Io(_))
        ));
        assert_eq!(base64_decode("aGk="), Some(b"hi".to_vec()));
        assert_eq!(base64_decode("!!"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn base64_encode_for_test(bytes: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
}
