//! PDF/X output (OpenCanvas patch, see `PATCHES.md`).
//!
//! krilla validates PDF/A and PDF/UA but has no notion of PDF/X. This module adds what a PDF/X file needs
//! at the document level, which is impossible to add from outside krilla without post-processing:
//!
//! - a `/OutputIntents` entry with `/S /GTS_PDFX`, the output condition identifier, registry, info and
//!   the destination output profile (the characterised printing condition, embedded as an ICC stream);
//! - `/GTS_PDFXVersion` (and `/GTS_PDFXConformance` for PDF/X-1a) plus `/Trapped` in the document
//!   information dictionary, and the matching XMP (`pdfxid:GTS_PDFXVersion`, `pdf:Trapped`,
//!   `xmpMM:VersionID`, `xmpMM:RenditionClass default`).
//!
//! Content-level rules (no RGB in X-1a, no transparency in X-1a, embedded fonts, TrimBox/BleedBox) are
//! the caller's responsibility: krilla only writes what it is given. OpenCanvas converts colours and
//! flattens transparency before drawing, and checks the result with a structural validator.

use std::sync::Arc;

/// Which PDF/X flavour to declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PdfXVersion {
    /// PDF/X-4 (ISO 15930-7): ICC-based colour and live transparency allowed, PDF 1.6.
    X4,
    /// PDF/X-1a:2003 (ISO 15930-4): CMYK/spot only, no transparency, PDF 1.4.
    X1a2003,
}

impl PdfXVersion {
    /// The `GTS_PDFXVersion` value.
    pub fn version_str(self) -> &'static str {
        match self {
            PdfXVersion::X4 => "PDF/X-4",
            PdfXVersion::X1a2003 => "PDF/X-1:2003",
        }
    }

    /// The `GTS_PDFXConformance` value, if the flavour has one.
    pub fn conformance_str(self) -> Option<&'static str> {
        match self {
            PdfXVersion::X4 => None,
            PdfXVersion::X1a2003 => Some("PDF/X-1a:2003"),
        }
    }
}

/// The characterised printing condition the file is prepared for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PdfXOutputIntent {
    /// The raw ICC profile bytes (output profile; CMYK `prtr` for print, or RGB).
    pub profile: Arc<Vec<u8>>,
    /// Number of colour components of the profile (4 for CMYK, 3 for RGB).
    pub components: u8,
    /// `/OutputConditionIdentifier`, e.g. `FOGRA51`.
    pub condition_identifier: String,
    /// `/OutputCondition`, a human readable description.
    pub condition: Option<String>,
    /// `/RegistryName`, normally `http://www.color.org`.
    pub registry_name: Option<String>,
    /// `/Info`, e.g. the profile description.
    pub info: Option<String>,
}

/// PDF/X settings for [`SerializeSettings::pdfx`](crate::SerializeSettings::pdfx).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PdfX {
    /// The flavour to declare.
    pub version: PdfXVersion,
    /// The characterised printing condition.
    pub output_intent: PdfXOutputIntent,
    /// The `/Trapped` key (PDF/X requires `True` or `False`).
    pub trapped: bool,
}
