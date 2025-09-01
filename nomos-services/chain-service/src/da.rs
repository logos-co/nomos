use nomos_core::block::{Height, SessionNumber};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy)]
pub enum BlobValidationPolicy {
    Skip,
    SkipOlderThan(Height),
}

impl BlobValidationPolicy {
    pub const fn skip_old(settings: &Settings, max_known_height: Height) -> Self {
        Self::SkipOlderThan(max_known_height.saturating_sub(settings.da_window_in_blocks()))
    }

    pub const fn should_validate(&self, height: Height) -> bool {
        match self {
            Self::Skip => false,
            Self::SkipOlderThan(min_height) => height >= *min_height,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Settings {
    /// The number of sessions for which DA nodes store blobs.
    pub availability_window_in_sessions: SessionNumber,
    /// The length of a session in blocks.
    /// Later, this should be included in the genesis block.
    pub session_length_in_blocks: Height,
}

impl Settings {
    #[must_use]
    pub const fn da_window_in_blocks(&self) -> Height {
        self.availability_window_in_sessions
            .checked_mul(self.session_length_in_blocks)
            .expect("Shouldn't overflow")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skip_old_policy() {
        let settings = Settings {
            availability_window_in_sessions: 2,
            session_length_in_blocks: 10,
        };

        let policy = BlobValidationPolicy::skip_old(&settings, 100);
        assert!(
            matches!(policy, BlobValidationPolicy::SkipOlderThan(80)),
            "{policy:?}"
        );

        let policy = BlobValidationPolicy::skip_old(&settings, 20);
        assert!(
            matches!(policy, BlobValidationPolicy::SkipOlderThan(0)),
            "{policy:?}"
        );

        let policy = BlobValidationPolicy::skip_old(&settings, 10);
        assert!(
            matches!(policy, BlobValidationPolicy::SkipOlderThan(0)),
            "{policy:?}"
        );
    }

    #[test]
    fn test_should_validate() {
        let policy = BlobValidationPolicy::Skip;
        assert!(!policy.should_validate(0));
        assert!(!policy.should_validate(100));

        let policy = BlobValidationPolicy::SkipOlderThan(50);
        assert!(!policy.should_validate(49));
        assert!(policy.should_validate(50));
        assert!(policy.should_validate(51));
    }
}
