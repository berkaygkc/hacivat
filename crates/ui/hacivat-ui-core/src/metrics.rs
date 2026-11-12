//! Pixel-grid helpers.

/// Snaps a logical length so it lands on a whole device pixel at `scale`.
pub fn snap_to_device_px(logical: f32, scale: f32) -> f32 {
    (logical * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_to_device_px_is_pixel_aligned() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for v in [0.3, 1.0, 20.5, 276.0] {
                let d = snap_to_device_px(v, scale) * scale;
                assert!((d - d.round()).abs() < 1e-4);
            }
        }
    }
}
