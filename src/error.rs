//! Crate-local error type used by `oxideav-pbm`'s standalone (no
//! `oxideav-core`) public API.
//!
//! When the `registry` feature is enabled, [`PbmError`] gains a
//! `From<PbmError> for oxideav_core::Error` impl (defined in
//! `crate::registry`) so the trait-side surface (`Decoder` /
//! `Encoder`) can keep returning `oxideav_core::Result<T>` while the
//! underlying decode/encode functions stay framework-free.

use core::fmt;

/// `Result` alias scoped to `oxideav-pbm`. Standalone (no `oxideav-core`)
/// callers see this; framework callers convert via the gated
/// `From<PbmError> for oxideav_core::Error` impl.
pub type Result<T> = core::result::Result<T, PbmError>;

/// The contract name for [`PbmError`] (`IMAGE_CRATE_API`).
pub type Error = PbmError;

/// Error variants returned by `oxideav-pbm`'s standalone API.
///
/// The variants mirror the subset of `oxideav_core::Error` the codec
/// can hit. The enum deliberately does not derive `Clone` /
/// `PartialEq` (it carries a [`std::io::Error`]); tests match on the
/// variant or on `Display`.
#[derive(Debug)]
#[non_exhaustive]
pub enum PbmError {
    /// The byte stream is malformed (bad magic, truncated header,
    /// non-numeric token where a sample was expected, …), or a
    /// caller-assembled image has inconsistent geometry.
    InvalidData(String),
    /// The byte stream uses a feature this codec doesn't implement,
    /// or the encoder was asked to emit a pixel format / option
    /// combination the Netpbm family cannot represent.
    Unsupported(String),
    /// A [`crate::DecodeOptions`] limit (`max_width` / `max_height` /
    /// `max_pixels` / `max_bytes`) would be exceeded; raised from the
    /// header, before any pixel buffer is allocated.
    LimitExceeded(String),
    /// An I/O error from [`crate::decode_from`] / [`crate::encode_to`].
    Io(std::io::Error),
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

    /// Construct a [`PbmError::LimitExceeded`] from a stringy message.
    pub fn limit(msg: impl Into<String>) -> Self {
        Self::LimitExceeded(msg.into())
    }

    /// `true` for [`PbmError::InvalidData`].
    pub fn is_invalid_data(&self) -> bool {
        matches!(self, Self::InvalidData(_))
    }

    /// `true` for [`PbmError::Unsupported`].
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported(_))
    }

    /// `true` for [`PbmError::LimitExceeded`].
    pub fn is_limit_exceeded(&self) -> bool {
        matches!(self, Self::LimitExceeded(_))
    }

    /// `true` for [`PbmError::Io`].
    pub fn is_io(&self) -> bool {
        matches!(self, Self::Io(_))
    }
}

impl fmt::Display for PbmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(s) => write!(f, "invalid data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::LimitExceeded(s) => write!(f, "limit exceeded: {s}"),
            Self::Io(e) => write!(f, "i/o error: {e}"),
        }
    }
}

impl std::error::Error for PbmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for PbmError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_prefixes_each_variant() {
        assert_eq!(
            PbmError::invalid("x").to_string(),
            "invalid data: x".to_string()
        );
        assert_eq!(PbmError::unsupported("y").to_string(), "unsupported: y");
        assert_eq!(PbmError::limit("z").to_string(), "limit exceeded: z");
        let io: PbmError = std::io::Error::other("boom").into();
        assert!(io.is_io());
        assert!(io.to_string().starts_with("i/o error: "));
        assert!(std::error::Error::source(&io).is_some());
    }
}
