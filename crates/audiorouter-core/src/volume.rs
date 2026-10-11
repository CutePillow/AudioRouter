//! Shared volume mapping for the user-facing 0-100% controls.
//!
//! The slider maps directly to PCM amplitude. This keeps the displayed
//! percentage predictable while preserving 100% as the unmodified signal and
//! 0% as silence on each endpoint.

pub const MAX_VOLUME_PERCENT: u8 = 100;

pub fn clamp_volume(volume: u8) -> u8 {
    volume.min(MAX_VOLUME_PERCENT)
}

pub fn volume_to_gain(volume: u8) -> f32 {
    clamp_volume(volume) as f32 / MAX_VOLUME_PERCENT as f32
}

#[cfg(test)]
mod tests {
    use super::{volume_to_gain, MAX_VOLUME_PERCENT};

    #[test]
    fn endpoints_are_silence_and_unmodified() {
        assert_eq!(volume_to_gain(0), 0.0);
        assert_eq!(volume_to_gain(MAX_VOLUME_PERCENT), 1.0);
    }

    #[test]
    fn slider_maps_to_linear_pcm_gain() {
        assert_eq!(volume_to_gain(50), 0.5);
        assert_eq!(volume_to_gain(80), 0.8);
        assert!(volume_to_gain(75) > volume_to_gain(50));
        assert!(volume_to_gain(99) < 1.0);
    }
}
