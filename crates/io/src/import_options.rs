//! Explicit authority for imports that may otherwise discover installed camera profiles.

/// Which camera calibration sources a caller authorizes while importing RAW pixels.
/// This is process authority supplied by the caller, never a value read from a document.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CameraProfilePolicy {
    /// Use only the embedded nine-profile bundle and the RAW's own calibration.
    /// No profile environment variables, user directories or external profile cache are read.
    #[default]
    BundledOnly,
    /// A trusted local caller authorizes explicitly configured and installed profiles.
    /// Invalid explicit overrides fail; unusable automatically discovered candidates are skipped.
    Installed,
}

/// Import behavior and authority. Defaults keep byte-only and recursive imports independent
/// of ambient camera-profile files. Trusted direct opens can opt into installed profiles.
#[derive(Clone, Copy, Debug)]
pub struct ImportOptions {
    /// SVG groups deeper than this threshold are rasterized during conversion.
    pub max_svg_group_depth: usize,
    /// Camera-profile sources authorized for this top-level RAW import.
    /// Embedded/recursive sources keep their own bundled-only default.
    pub camera_profiles: CameraProfilePolicy,
}

impl Default for ImportOptions {
    /// Apply the document's group-depth limit and authorize only bundled camera profiles.
    fn default() -> Self {
        Self { max_svg_group_depth: photocraft_doc::MAX_GROUP_DEPTH, camera_profiles: CameraProfilePolicy::BundledOnly }
    }
}
