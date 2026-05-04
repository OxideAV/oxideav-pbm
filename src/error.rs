//! Crate-local error type used by `oxideav-pbm`'s standalone (no
//! `oxideav-core`) public API.
//!
//! When the `registry` feature is enabled, [`PbmError`] gains a
//! `From<PbmError> for oxideav_core::Error` impl (defined in
//! [`crate::registry`]) so the trait-side surface (`Decoder` /
//! `Encoder`) can keep returning `oxideav_core::Result<T>` while the
//! underlying decode/encode functions stay framework-free.

use core::fmt;

/// `Result` alias scoped to `oxideav-pbm`. Standalone (no `oxideav-core`)
/// callers see this; framework callers convert via the gated
/// `From<PbmError> for oxideav_core::Error` impl.
pub type Result<T> = core::result::Result<T, PbmError>;

/// Error variants returned by `oxideav-pbm`'s standalone API.
///
/// The variants mirror the subset of `oxideav_core::Error` the codec
/// can hit. The crate intentionally avoids surfacing transport (`Io`)
/// or framework-specific (`FormatNotFound`, `CodecNotFound`) errors —
/// those originate in callers that are already linking `oxideav-core`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PbmError {
    /// The byte stream is malformed (bad magic, truncated header,
    /// non-numeric token where a sample was expected, …).
    InvalidData(String),
    /// The byte stream uses a feature this codec doesn't implement,
    /// or the encoder was asked to emit a pixel format it doesn't
    /// support.
    Unsupported(String),
}

impl PbmError {
    /// Construct a [`PbmError::InvalidData`] from a stringy message.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidData(msg.into())
    }

    /// Construct a [`PbmError::Unsupported`] from a stringy message.
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }
}

impl fmt::Display for PbmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(s) => write!(f, "invalid data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for PbmError {}
