//! Strict sequential `Content-Range` parsing and resumable upload state.
//!
//! File Tunnel accepts chunks in order. Exact retries of an already committed
//! chunk are idempotent when their bytes match; gaps, partial overlaps, total
//! changes, and conflicting retries fail closed.

use std::error::Error;
use std::fmt::{Display, Formatter};

/// A validated byte range with an exclusive end offset.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentRange {
    /// First byte in the request body.
    pub start: u64,
    /// Offset immediately after the final byte in the request body.
    pub end_exclusive: u64,
    /// Declared complete object size.
    pub total: u64,
}

impl ContentRange {
    /// Creates the implicit range used by a legacy single-request upload.
    #[must_use]
    pub const fn whole(total: u64) -> Self {
        Self {
            start: 0,
            end_exclusive: total,
            total,
        }
    }

    /// Parses `Content-Range: bytes START-END/TOTAL` for a previously declared
    /// object size.
    ///
    /// Wildcards and unknown totals are deliberately rejected because the file
    /// declaration already supplies the authoritative size.
    pub fn parse(value: &str, expected_total: u64) -> Result<Self, RangeError> {
        let value = value.trim();
        let remainder = value
            .strip_prefix("bytes ")
            .ok_or(RangeError::InvalidUnit)?;
        let (bounds, total) = remainder
            .split_once('/')
            .ok_or(RangeError::InvalidSyntax)?;
        if total.contains('/') || bounds.contains(',') || total == "*" || bounds == "*" {
            return Err(RangeError::UnsupportedWildcard);
        }
        let total = parse_u64(total).ok_or(RangeError::InvalidNumber)?;
        if total != expected_total {
            return Err(RangeError::TotalMismatch {
                expected: expected_total,
                actual: total,
            });
        }
        let (start, end_inclusive) = bounds
            .split_once('-')
            .ok_or(RangeError::InvalidSyntax)?;
        if end_inclusive.contains('-') {
            return Err(RangeError::InvalidSyntax);
        }
        let start = parse_u64(start).ok_or(RangeError::InvalidNumber)?;
        let end_inclusive = parse_u64(end_inclusive).ok_or(RangeError::InvalidNumber)?;
        if end_inclusive < start {
            return Err(RangeError::Reversed);
        }
        let end_exclusive = end_inclusive
            .checked_add(1)
            .ok_or(RangeError::OffsetOverflow)?;
        if end_exclusive > total {
            return Err(RangeError::OutsideDeclaredSize {
                end_exclusive,
                total,
            });
        }
        Ok(Self {
            start,
            end_exclusive,
            total,
        })
    }

    /// Number of bytes required in the request body.
    #[must_use]
    pub const fn len(self) -> u64 {
        self.end_exclusive - self.start
    }

    /// Reports whether this range carries no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// Strict `Content-Range` parse failure.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RangeError {
    InvalidUnit,
    InvalidSyntax,
    InvalidNumber,
    UnsupportedWildcard,
    Reversed,
    OffsetOverflow,
    TotalMismatch { expected: u64, actual: u64 },
    OutsideDeclaredSize { end_exclusive: u64, total: u64 },
}

impl Display for RangeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUnit => formatter.write_str("Content-Range must use the `bytes` unit"),
            Self::InvalidSyntax => formatter.write_str(
                "Content-Range must use `bytes START-END/TOTAL` syntax",
            ),
            Self::InvalidNumber => {
                formatter.write_str("Content-Range offsets must be unsigned decimal integers")
            }
            Self::UnsupportedWildcard => {
                formatter.write_str("Content-Range wildcards and multipart ranges are unsupported")
            }
            Self::Reversed => formatter.write_str("Content-Range end precedes its start"),
            Self::OffsetOverflow => formatter.write_str("Content-Range end offset overflowed"),
            Self::TotalMismatch { expected, actual } => write!(
                formatter,
                "Content-Range total {actual} does not match declared size {expected}"
            ),
            Self::OutsideDeclaredSize {
                end_exclusive,
                total,
            } => write!(
                formatter,
                "Content-Range ends at {end_exclusive}, beyond declared size {total}"
            ),
        }
    }
}

impl Error for RangeError {}

/// Result of committing one request body to an upload buffer.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct AppendOutcome {
    /// Next byte offset expected by the server.
    pub offset: u64,
    /// Whether all declared bytes are present.
    pub complete: bool,
    /// Whether this was an exact idempotent replay rather than new progress.
    pub replayed: bool,
}

/// In-memory sequential upload state used by the reference server.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UploadBuffer {
    total: u64,
    bytes: Vec<u8>,
}

impl UploadBuffer {
    /// Starts an empty upload for a declared object size.
    #[must_use]
    pub const fn new(total: u64) -> Self {
        Self {
            total,
            bytes: Vec::new(),
        }
    }

    /// Returns the declared complete size.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.total
    }

    /// Returns the next byte offset expected by the server.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Reports whether every declared byte has been committed.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.offset() == self.total
    }

    /// Returns committed bytes in wire order.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Commits one validated request body.
    ///
    /// New data must begin exactly at [`Self::offset`]. A range fully contained
    /// in committed data is accepted only when every replayed byte matches.
    pub fn append(
        &mut self,
        range: ContentRange,
        chunk: &[u8],
    ) -> Result<AppendOutcome, UploadError> {
        if range.total != self.total {
            return Err(UploadError::TotalChanged {
                expected: self.total,
                actual: range.total,
            });
        }
        let expected_len = usize::try_from(range.len()).map_err(|_| UploadError::ChunkTooLarge)?;
        if chunk.len() != expected_len {
            return Err(UploadError::BodyLength {
                expected: range.len(),
                actual: chunk.len() as u64,
            });
        }

        let offset = self.offset();
        if range.start > offset {
            return Err(UploadError::Gap {
                expected_start: offset,
                actual_start: range.start,
            });
        }
        if range.start < offset {
            if range.end_exclusive > offset {
                return Err(UploadError::PartialOverlap {
                    committed: offset,
                    start: range.start,
                    end_exclusive: range.end_exclusive,
                });
            }
            let start = usize::try_from(range.start).map_err(|_| UploadError::ChunkTooLarge)?;
            let end = usize::try_from(range.end_exclusive)
                .map_err(|_| UploadError::ChunkTooLarge)?;
            if self.bytes.get(start..end) != Some(chunk) {
                return Err(UploadError::ConflictingReplay {
                    start: range.start,
                    end_exclusive: range.end_exclusive,
                });
            }
            return Ok(AppendOutcome {
                offset,
                complete: self.is_complete(),
                replayed: true,
            });
        }

        self.bytes.extend_from_slice(chunk);
        let offset = self.offset();
        debug_assert!(offset <= self.total);
        Ok(AppendOutcome {
            offset,
            complete: self.is_complete(),
            replayed: false,
        })
    }
}

/// Sequential upload state-transition failure.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UploadError {
    TotalChanged {
        expected: u64,
        actual: u64,
    },
    BodyLength {
        expected: u64,
        actual: u64,
    },
    Gap {
        expected_start: u64,
        actual_start: u64,
    },
    PartialOverlap {
        committed: u64,
        start: u64,
        end_exclusive: u64,
    },
    ConflictingReplay {
        start: u64,
        end_exclusive: u64,
    },
    ChunkTooLarge,
}

impl Display for UploadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TotalChanged { expected, actual } => write!(
                formatter,
                "upload total changed from {expected} to {actual}"
            ),
            Self::BodyLength { expected, actual } => write!(
                formatter,
                "request body contains {actual} bytes; Content-Range requires {expected}"
            ),
            Self::Gap {
                expected_start,
                actual_start,
            } => write!(
                formatter,
                "upload gap: server expects offset {expected_start}, request begins at {actual_start}"
            ),
            Self::PartialOverlap {
                committed,
                start,
                end_exclusive,
            } => write!(
                formatter,
                "range {start}..{end_exclusive} partially overlaps committed offset {committed}"
            ),
            Self::ConflictingReplay {
                start,
                end_exclusive,
            } => write!(
                formatter,
                "replayed range {start}..{end_exclusive} does not match committed bytes"
            ),
            Self::ChunkTooLarge => {
                formatter.write_str("range length cannot be represented by this server")
            }
        }
    }
}

impl Error for UploadError {}

fn parse_u64(value: &str) -> Option<u64> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_strict_content_range() {
        assert_eq!(
            ContentRange::parse("bytes 10-19/100", 100),
            Ok(ContentRange {
                start: 10,
                end_exclusive: 20,
                total: 100,
            })
        );
        assert_eq!(ContentRange::parse("items 0-9/10", 10), Err(RangeError::InvalidUnit));
        assert_eq!(
            ContentRange::parse("bytes 0-9/11", 10),
            Err(RangeError::TotalMismatch {
                expected: 10,
                actual: 11,
            })
        );
        assert!(ContentRange::parse("bytes */10", 10).is_err());
        assert!(ContentRange::parse("bytes 01-9/10", 10).is_err());
    }

    #[test]
    fn appends_sequential_chunks_to_completion() {
        let mut upload = UploadBuffer::new(6);
        assert_eq!(
            upload.append(
                ContentRange::parse("bytes 0-2/6", 6).expect("first range"),
                b"abc",
            ),
            Ok(AppendOutcome {
                offset: 3,
                complete: false,
                replayed: false,
            })
        );
        assert_eq!(
            upload.append(
                ContentRange::parse("bytes 3-5/6", 6).expect("second range"),
                b"def",
            ),
            Ok(AppendOutcome {
                offset: 6,
                complete: true,
                replayed: false,
            })
        );
        assert_eq!(upload.as_slice(), b"abcdef");
    }

    #[test]
    fn exact_retry_is_idempotent() {
        let mut upload = UploadBuffer::new(6);
        let range = ContentRange::parse("bytes 0-2/6", 6).expect("range");
        upload.append(range, b"abc").expect("first append");
        assert_eq!(
            upload.append(range, b"abc"),
            Ok(AppendOutcome {
                offset: 3,
                complete: false,
                replayed: true,
            })
        );
        assert_eq!(upload.as_slice(), b"abc");
    }

    #[test]
    fn conflicting_retry_and_partial_overlap_fail_closed() {
        let mut upload = UploadBuffer::new(6);
        upload
            .append(ContentRange::parse("bytes 0-2/6", 6).expect("range"), b"abc")
            .expect("first append");
        assert!(matches!(
            upload.append(
                ContentRange::parse("bytes 0-2/6", 6).expect("retry"),
                b"xyz"
            ),
            Err(UploadError::ConflictingReplay { .. })
        ));
        assert!(matches!(
            upload.append(
                ContentRange::parse("bytes 2-4/6", 6).expect("overlap"),
                b"cde"
            ),
            Err(UploadError::PartialOverlap { .. })
        ));
        assert_eq!(upload.as_slice(), b"abc");
    }

    #[test]
    fn gaps_and_wrong_body_lengths_do_not_mutate_state() {
        let mut upload = UploadBuffer::new(6);
        assert!(matches!(
            upload.append(
                ContentRange::parse("bytes 3-5/6", 6).expect("gap"),
                b"def"
            ),
            Err(UploadError::Gap { .. })
        ));
        assert!(matches!(
            upload.append(
                ContentRange::parse("bytes 0-2/6", 6).expect("length"),
                b"ab"
            ),
            Err(UploadError::BodyLength { .. })
        ));
        assert_eq!(upload.offset(), 0);
    }

    #[test]
    fn empty_legacy_upload_completes() {
        let mut upload = UploadBuffer::new(0);
        assert_eq!(
            upload.append(ContentRange::whole(0), b""),
            Ok(AppendOutcome {
                offset: 0,
                complete: true,
                replayed: false,
            })
        );
    }
}
