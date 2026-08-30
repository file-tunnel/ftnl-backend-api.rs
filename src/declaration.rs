//! Pure file-declaration policy.
//!
//! HTTP extraction, capability checks, locks, clocks, identifiers, storage,
//! and event publication remain in the Axum shell. This module receives an
//! immutable policy snapshot and command, then returns a typed decision.

/// Immutable limits visible to one declaration decision.
#[derive(Debug, Clone, Copy)]
pub struct DeclarationPolicy<'a> {
    pub current_files: usize,
    pub max_files: u16,
    pub max_file_bytes: u64,
    pub accepted_media: &'a [String],
}

/// Borrowed, side-effect-free declaration input.
#[derive(Debug, Clone, Copy)]
pub struct FileDeclaration<'a> {
    pub name: &'a str,
    pub media_type: &'a str,
    pub size_bytes: u64,
    pub sha256: Option<&'a str>,
}

/// Exhaustive reasons a declaration cannot enter the data plane.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DeclarationRejection {
    InvalidName,
    FileLimitReached,
    FileTooLarge,
    UnsupportedMedia,
    InvalidSha256,
}

/// Decide whether a file declaration satisfies the supplied policy snapshot.
///
/// No input is mutated and no observable effect occurs. Callers can therefore
/// exercise this decision independently from the web server and map its typed
/// rejection into their transport-specific error contract.
pub fn validate_declaration(
    policy: DeclarationPolicy<'_>,
    declaration: FileDeclaration<'_>,
) -> Result<(), DeclarationRejection> {
    if declaration.name.is_empty()
        || declaration.name.len() > 255
        || declaration
            .name
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        return Err(DeclarationRejection::InvalidName);
    }
    if policy.current_files >= usize::from(policy.max_files) {
        return Err(DeclarationRejection::FileLimitReached);
    }
    if declaration.size_bytes > policy.max_file_bytes {
        return Err(DeclarationRejection::FileTooLarge);
    }
    if !media_is_accepted(policy.accepted_media, declaration.media_type) {
        return Err(DeclarationRejection::UnsupportedMedia);
    }
    if declaration.sha256.is_some_and(|digest| {
        digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(DeclarationRejection::InvalidSha256);
    }
    Ok(())
}

fn media_is_accepted(patterns: &[String], media_type: &str) -> bool {
    patterns.iter().any(|pattern| {
        pattern == "*/*"
            || pattern == media_type
            || pattern
                .strip_suffix("/*")
                .is_some_and(|prefix| media_type.starts_with(&format!("{prefix}/")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy<'a>(accepted_media: &'a [String]) -> DeclarationPolicy<'a> {
        DeclarationPolicy {
            current_files: 0,
            max_files: 2,
            max_file_bytes: 1024,
            accepted_media,
        }
    }

    fn declaration<'a>(name: &'a str, media_type: &'a str) -> FileDeclaration<'a> {
        FileDeclaration {
            name,
            media_type,
            size_bytes: 512,
            sha256: None,
        }
    }

    #[test]
    fn accepts_exact_wildcard_and_global_media_policies() {
        for (patterns, media_type) in [
            (vec!["application/pdf".to_owned()], "application/pdf"),
            (vec!["image/*".to_owned()], "image/jpeg"),
            (vec!["*/*".to_owned()], "text/plain"),
        ] {
            assert_eq!(
                validate_declaration(policy(&patterns), declaration("file.bin", media_type)),
                Ok(())
            );
        }
    }

    #[test]
    fn returns_one_typed_rejection_for_each_policy_boundary() {
        let image_only = vec!["image/*".to_owned()];
        let valid = declaration("photo.jpg", "image/jpeg");
        let cases = [
            (
                policy(&image_only),
                declaration("bad\nname", "image/jpeg"),
                DeclarationRejection::InvalidName,
            ),
            (
                DeclarationPolicy {
                    current_files: 2,
                    ..policy(&image_only)
                },
                valid,
                DeclarationRejection::FileLimitReached,
            ),
            (
                policy(&image_only),
                FileDeclaration {
                    size_bytes: 1025,
                    ..valid
                },
                DeclarationRejection::FileTooLarge,
            ),
            (
                policy(&image_only),
                declaration("notes.txt", "text/plain"),
                DeclarationRejection::UnsupportedMedia,
            ),
            (
                policy(&image_only),
                FileDeclaration {
                    sha256: Some("not-a-sha256"),
                    ..valid
                },
                DeclarationRejection::InvalidSha256,
            ),
        ];

        for (policy, declaration, expected) in cases {
            assert_eq!(validate_declaration(policy, declaration), Err(expected));
        }
    }
}
