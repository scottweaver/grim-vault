//! This app's own JSON interchange documents — the blueprint and
//! illusion exports that carry a list from one campaign to another —
//! share one envelope: a `format` tag naming the document kind and a
//! `version`, checked before the rest is read so a foreign document is
//! refused by name rather than by a confusing shape error. Every such
//! document also names the campaign it was taken from and when: a
//! list moved between folders stays self-describing (the global rule
//! that a serialized fact carries its full identity). Unknown
//! top-level fields survive a read/write cycle, as the vault store's
//! do.

use std::fmt;

use serde::Deserialize;
use thiserror::Error;

/// Why an interchange document was refused before its body was read.
#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("not a {expected} document: format tag {found:?}")]
    WrongFormat {
        expected: &'static str,
        found: String,
    },
    #[error("{format} document version {version} is newer than this app's {supported}")]
    UnsupportedVersion {
        format: &'static str,
        version: u32,
        supported: u32,
    },
    #[error("document JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Deserialize)]
struct Envelope {
    format: String,
    version: u32,
}

/// Admits `bytes` as a `format` document at or below `supported`.
///
/// # Errors
/// [`EnvelopeError`].
pub fn check(bytes: &[u8], format: &'static str, supported: u32) -> Result<(), EnvelopeError> {
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    if envelope.format != format {
        return Err(EnvelopeError::WrongFormat {
            expected: format,
            found: envelope.format,
        });
    }
    if envelope.version > supported {
        return Err(EnvelopeError::UnsupportedVersion {
            format,
            version: envelope.version,
            supported,
        });
    }
    Ok(())
}

/// What an import did with each entry of a document: added, skipped as
/// already present, or refused for a reason the caller can show. An
/// import never removes anything and never stops at a refusal.
#[derive(Debug)]
pub struct ImportReport<E> {
    pub added: usize,
    pub already_known: usize,
    pub refused: Vec<(String, E)>,
}

impl<E> Default for ImportReport<E> {
    fn default() -> Self {
        Self {
            added: 0,
            already_known: 0,
            refused: Vec::new(),
        }
    }
}

impl<E> ImportReport<E> {
    /// Whether anything was added, hence whether the file needs saving.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.added > 0
    }
}

impl<E: fmt::Display> fmt::Display for ImportReport<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} added, {} already known, {} refused",
            self.added,
            self.already_known,
            self.refused.len()
        )?;
        for (record, reason) in &self.refused {
            write!(f, "\n  {record}: {reason}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_envelope_is_checked_by_tag_then_version() {
        assert!(check(br#"{"format":"x","version":1,"more":[]}"#, "x", 1).is_ok());
        assert!(check(br#"{"format":"x","version":0}"#, "x", 1).is_ok());
        assert!(matches!(
            check(br#"{"format":"y","version":1}"#, "x", 1),
            Err(EnvelopeError::WrongFormat { expected: "x", found }) if found == "y"
        ));
        assert!(matches!(
            check(br#"{"format":"x","version":2}"#, "x", 1),
            Err(EnvelopeError::UnsupportedVersion {
                version: 2,
                supported: 1,
                ..
            })
        ));
        assert!(matches!(check(b"{", "x", 1), Err(EnvelopeError::Json(_))));
        assert!(matches!(
            check(br#"{"version":1}"#, "x", 1),
            Err(EnvelopeError::Json(_))
        ));
    }

    #[test]
    fn a_report_says_what_changed() {
        let mut report = ImportReport::<String>::default();
        assert!(!report.changed());
        report.added = 2;
        report.already_known = 1;
        report
            .refused
            .push(("records/x.dbr".into(), "no such record".into()));
        assert!(report.changed());
        assert_eq!(
            report.to_string(),
            "2 added, 1 already known, 1 refused\n  records/x.dbr: no such record"
        );
    }
}
