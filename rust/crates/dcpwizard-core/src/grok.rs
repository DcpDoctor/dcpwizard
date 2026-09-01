//! TIFF loading for the in-process JPEG 2000 encoder.

pub use postkit::grok::{TiffFrame, load_tiff};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_tiff_missing_file() {
        let result = load_tiff(std::path::Path::new("/nonexistent.tif"));
        assert!(result.is_err());
    }
}
