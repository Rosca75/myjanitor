//! Row and card badges (spec §8 "clear status badges").

use crate::flags::flag_set;

flag_set! {
    /// Badges shown on candidates and opportunity cards. Serialised as an array of names.
    /// `confidential` and `protected` imply `default_selected = false`.
    pub struct Badges: u32 {
        /// Member of a duplicate group.
        DUPLICATE = 1 << 0 => "duplicate",
        /// Recommended copy to keep in its group (never selected by default).
        KEEPER = 1 << 1 => "keeper",
        /// The action on this row loses information.
        LOSSY = 1 << 2 => "lossy",
        /// Encrypted or password protected (SAF-07).
        ENCRYPTED = 1 << 3 => "encrypted",
        /// Digitally signed PDF/Office file (SAF-09).
        SIGNED = 1 << 4 => "signed",
        /// Carries a sensitivity label (SAF-07/08).
        LABELLED = 1 << 5 => "labelled",
        /// Inside a synced folder (OneDrive/SharePoint/Dropbox...): deletion propagates (SAF-10).
        SYNCED = 1 << 6 => "synced",
        /// Cloud placeholder whose content is not local (SCAN-09).
        ONLINE_ONLY = 1 << 7 => "online_only",
        /// Under a protected path: shown, never actionable (SAF-01).
        PROTECTED = 1 << 8 => "protected",
        /// Confidential by rule (labelled, encrypted, signed, privileged): auto-unchecked.
        CONFIDENTIAL = 1 << 9 => "confidential",
        /// Already a hard link to another file (no space to gain by deleting one copy).
        HARDLINKED = 1 << 10 => "hardlinked",
        /// Regenerable by a tool (dev artefacts, caches).
        REBUILDABLE = 1 << 11 => "rebuildable",
        /// The file appears to be open or locked by another program.
        IN_USE = 1 << 12 => "in_use",
    }
}

impl Badges {
    /// Badges that force `default_selected = false`.
    pub const AUTO_UNCHECK: Badges = Badges(
        Badges::CONFIDENTIAL.0
            | Badges::PROTECTED.0
            | Badges::ENCRYPTED.0
            | Badges::SIGNED.0
            | Badges::LABELLED.0
            | Badges::KEEPER.0
            | Badges::IN_USE.0,
    );

    /// Badges that make a candidate non-actionable whatever the user selects.
    pub const BLOCKING: Badges = Badges(Badges::PROTECTED.0);
}
