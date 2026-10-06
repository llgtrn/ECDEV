//! Licence policy, three questions kept apart. A donor's declared licence string decides what
//! ECDEV may do with its code (adoption), with its knowledge (study), and how a native version may
//! come to exist (reimplementation). These are governance policies read off the declared string,
//! never a legal conclusion: an unverified or unrecognised licence gets the most restrictive code
//! policy, and study stays permitted because reading behaviour, documentation and benchmarks
//! copies nothing into ECDEV.

/// The licence family a declared licence string falls in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Permissive,
    WeakCopyleft,
    StrongCopyleft,
    NonCommercial,
    Unverified,
}

/// May donor code (text) enter ECDEV?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeAdoption {
    /// Permissive: code may be adopted, keeping its notice.
    AdoptWithNotice,
    /// Weak copyleft: not copied without a file-level review.
    NotCopiedWithoutReview,
    /// Strong copyleft or non-commercial: never copied.
    NotCopied,
    /// Licence unverified: never copied until it is.
    NotCopiedUntilVerified,
}

/// What ECDEV may learn from the donor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnowledgeStudy {
    /// Study, benchmark, and derive native code from it.
    StudyAndDerive,
    /// Study behaviour, documentation, algorithms and outputs, and benchmark against them, as an
    /// external reference; no code, text or data copied.
    StudyReadOnly,
}

/// How a native implementation of a donor capability may come to exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeReimplementation {
    /// Derived from the donor (DERIVED_NATIVE), with attribution.
    DerivedWithAttribution,
    /// Only independently, from documented behaviour (INDEPENDENT_NATIVE or a deliberate
    /// divergence); a DERIVED_NATIVE relation is refused.
    CleanRoomOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub family: Family,
    pub code_adoption: CodeAdoption,
    pub knowledge_study: KnowledgeStudy,
    pub native_reimplementation: NativeReimplementation,
}

pub const BASIS: &str =
    "read off the declared licence string; a governance policy, not a legal conclusion";

const PERMISSIVE: &[&str] = &[
    "MIT",
    "APACHE-2.0",
    "BSD-2-CLAUSE",
    "BSD-3-CLAUSE",
    "ISC",
    "ZLIB",
    "UNLICENSE",
    "0BSD",
    "PSF-2.0",
    "CC0-1.0",
    "BLESSING",
    "PUBLIC-DOMAIN",
];

impl Family {
    pub fn of(licence: &str) -> Family {
        let u = licence.to_ascii_uppercase();
        let has = |w: &str| u.contains(w);
        if has("NON-COMMERCIAL") || has("NONCOMMERCIAL") || has("-NC") || has("POLYFORM") {
            Family::NonCommercial
        } else if has("AGPL") || (has("GPL") && !has("LGPL")) || has("SSPL") {
            Family::StrongCopyleft
        } else if has("LGPL") || has("MPL") || has("EPL") || has("CDDL") {
            Family::WeakCopyleft
        } else {
            let tokens: Vec<&str> = u
                .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
                .filter(|t| !t.is_empty() && *t != "OR" && *t != "AND")
                .collect();
            if !tokens.is_empty() && tokens.iter().all(|t| PERMISSIVE.contains(t)) {
                Family::Permissive
            } else {
                Family::Unverified
            }
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Family::Permissive => "PERMISSIVE",
            Family::WeakCopyleft => "WEAK_COPYLEFT",
            Family::StrongCopyleft => "STRONG_COPYLEFT",
            Family::NonCommercial => "NON_COMMERCIAL",
            Family::Unverified => "UNVERIFIED",
        }
    }
}

impl CodeAdoption {
    pub fn word(self) -> &'static str {
        match self {
            CodeAdoption::AdoptWithNotice => "ADOPT_WITH_NOTICE",
            CodeAdoption::NotCopiedWithoutReview => "NOT_COPIED_WITHOUT_REVIEW",
            CodeAdoption::NotCopied => "NOT_COPIED",
            CodeAdoption::NotCopiedUntilVerified => "NOT_COPIED_UNTIL_VERIFIED",
        }
    }
}

impl KnowledgeStudy {
    pub fn word(self) -> &'static str {
        match self {
            KnowledgeStudy::StudyAndDerive => "STUDY_AND_DERIVE",
            KnowledgeStudy::StudyReadOnly => "STUDY_READ_ONLY",
        }
    }
}

impl NativeReimplementation {
    pub fn word(self) -> &'static str {
        match self {
            NativeReimplementation::DerivedWithAttribution => "DERIVED_WITH_ATTRIBUTION",
            NativeReimplementation::CleanRoomOnly => "CLEAN_ROOM_ONLY",
        }
    }
}

/// The three policies of a declared licence. Study is never forbidden by a licence: a
/// restrictive licence narrows what may be copied and how a native version is built, not what
/// may be learned.
pub fn policy(licence: &str) -> Policy {
    let family = Family::of(licence);
    let (code_adoption, knowledge_study, native_reimplementation) = match family {
        Family::Permissive => (
            CodeAdoption::AdoptWithNotice,
            KnowledgeStudy::StudyAndDerive,
            NativeReimplementation::DerivedWithAttribution,
        ),
        Family::WeakCopyleft => (
            CodeAdoption::NotCopiedWithoutReview,
            KnowledgeStudy::StudyReadOnly,
            NativeReimplementation::CleanRoomOnly,
        ),
        Family::StrongCopyleft | Family::NonCommercial => (
            CodeAdoption::NotCopied,
            KnowledgeStudy::StudyReadOnly,
            NativeReimplementation::CleanRoomOnly,
        ),
        Family::Unverified => (
            CodeAdoption::NotCopiedUntilVerified,
            KnowledgeStudy::StudyReadOnly,
            NativeReimplementation::CleanRoomOnly,
        ),
    };
    Policy {
        family,
        code_adoption,
        knowledge_study,
        native_reimplementation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_of_declared_licences() {
        for (l, f) in [
            ("MIT", Family::Permissive),
            ("MIT OR Apache-2.0", Family::Permissive),
            ("(Apache-2.0 OR MIT) AND BSD-3-Clause", Family::Permissive),
            ("GPL-2.0", Family::StrongCopyleft),
            (
                "AGPL-3.0 (root); component licenses recorded separately",
                Family::StrongCopyleft,
            ),
            ("LGPL-2.1", Family::WeakCopyleft),
            ("NON-COMMERCIAL-LEARNING-1.1", Family::NonCommercial),
            ("UNKNOWN", Family::Unverified),
            ("UNVERIFIED", Family::Unverified),
            ("", Family::Unverified),
            ("MIT AND Proprietary", Family::Unverified),
            ("blessing", Family::Permissive),
        ] {
            assert_eq!(Family::of(l), f, "{l}");
        }
    }

    #[test]
    fn no_licence_forbids_study() {
        for l in [
            "GPL-3.0",
            "AGPL-3.0",
            "NON-COMMERCIAL-LEARNING-1.1",
            "UNKNOWN",
            "MPL-2.0",
        ] {
            let p = policy(l);
            assert_eq!(p.knowledge_study, KnowledgeStudy::StudyReadOnly, "{l}");
            assert_eq!(
                p.native_reimplementation,
                NativeReimplementation::CleanRoomOnly,
                "{l}"
            );
            assert_ne!(p.code_adoption, CodeAdoption::AdoptWithNotice, "{l}");
        }
        assert_eq!(policy("MIT").code_adoption, CodeAdoption::AdoptWithNotice);
    }
}
