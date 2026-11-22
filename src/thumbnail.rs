//! Bounded thumbnail decoding and one attempt per active popup URL.

#[derive(Default)]
pub struct Attempt {
    url: Option<String>,
}

impl Attempt {
    /// Mark before starting the worker so failures never cause repaint retries.
    pub fn begin(&mut self, url: &str) -> bool {
        if self.url.as_deref() == Some(url) {
            return false;
        }
        self.url = Some(url.to_owned());
        true
    }
}

pub fn decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    Some((width, height, image.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    #[test]
    fn failed_or_disconnected_attempt_is_not_retried_until_popup_changes() {
        let mut attempt = Attempt::default();
        assert!(attempt.begin("https://example.invalid/missing.png"));
        for _ in 0..10 {
            assert!(!attempt.begin("https://example.invalid/missing.png"));
        }
        assert!(attempt.begin("https://example.invalid/other.png"));
        attempt = Attempt::default();
        assert!(attempt.begin("https://example.invalid/missing.png"));
    }

    #[test]
    fn decoded_dimensions_are_bounded_even_when_compressed_input_is_small() {
        for (width, expected) in [(2, true), (2049, false)] {
            let mut bytes = Vec::new();
            image::codecs::png::PngEncoder::new(&mut bytes)
                .write_image(
                    &vec![0; width as usize * 4],
                    width,
                    1,
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
            assert!(bytes.len() < 3 * 1024 * 1024);
            assert_eq!(decode(&bytes).is_some(), expected);
        }
        assert!(decode(b"invalid image").is_none());
    }
}
