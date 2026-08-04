/// Where a migration has got to.
///
/// Ordered as the migration performs them. `Committed` is the only terminal
/// success; every other value names a place the process can be found stopped.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The state exists, the destination does not. Nothing has been moved.
    Prepared,
    /// The destination is built and has been checked against the source.
    DestinationValidated,
    /// The source has been moved aside, under its archive name.
    SourceArchived,
    /// The destination now sits at the source's name.
    DestinationActivated,
    /// The archive has been released. The migration is over.
    Committed,
}

/// Every phase, in order — so an exhaustive check cannot silently miss one
/// added later.
pub const PHASES: &[Phase] = &[
    Phase::Prepared,
    Phase::DestinationValidated,
    Phase::SourceArchived,
    Phase::DestinationActivated,
    Phase::Committed,
];

/// What to do with a migration found stopped.
///
/// There is deliberately no "clean up and carry on" variant. Every outcome
/// either moves forward from a known point, puts the source back, refuses, or
/// hands over to a human — and the last two change nothing on disk.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "recovery", rename_all = "snake_case")]
pub enum Recovery {
    /// Resume forward, starting at the named phase.
    Continue {
        /// The phase to perform next.
        next: Phase,
        /// Why continuing is safe from here.
        rationale: String,
    },
    /// Put the source back where it was. Only ever from a state where the
    /// source is provably intact.
    Restore {
        /// Exactly what to move where.
        action: String,
    },
    /// Change nothing. The state on disk does not determine what happened.
    Refuse {
        /// What is ambiguous, and what a human must look at.
        reason: String,
    },
}

impl Phase {
    /// Whether `self` is a permitted journal update after `previous`.
    ///
    /// Rewriting the same phase is idempotent. Advancing exactly one phase is
    /// permitted. Skips and regressions are refused so the journal can never
    /// claim that an unrecorded destructive step happened.
    pub(super) fn may_follow(self, previous: Self) -> bool {
        match previous {
            Self::Prepared => matches!(self, Self::Prepared | Self::DestinationValidated),
            Self::DestinationValidated => {
                matches!(self, Self::DestinationValidated | Self::SourceArchived)
            }
            Self::SourceArchived => {
                matches!(self, Self::SourceArchived | Self::DestinationActivated)
            }
            Self::DestinationActivated => {
                matches!(self, Self::DestinationActivated | Self::Committed)
            }
            Self::Committed => matches!(self, Self::Committed),
        }
    }

    /// What to do when a migration is found stopped in this phase.
    ///
    /// Total by construction — the match has no wildcard arm, so a phase added
    /// later fails to compile until its action is decided rather than
    /// inheriting someone else's.
    #[must_use]
    pub fn recovery(self) -> Recovery {
        match self {
            Self::Prepared => Recovery::Continue {
                next: Self::DestinationValidated,
                rationale: "nothing has been moved: the destination is built from the source, \
                            which is still in place and still authoritative."
                    .to_owned(),
            },
            Self::DestinationValidated => Recovery::Continue {
                next: Self::SourceArchived,
                rationale: "the destination is built and checked, and the source is untouched. \
                            The next step is the first that moves anything."
                    .to_owned(),
            },
            Self::SourceArchived => Recovery::Restore {
                action: "move the archive back to the source name. The destination was never \
                         activated, so the source is the only authority and putting it back is \
                         the whole recovery."
                    .to_owned(),
            },
            Self::DestinationActivated => Recovery::Continue {
                next: Self::Committed,
                rationale: "the destination is in place and the archive still exists. Going \
                            forward releases the archive; going back would discard a destination \
                            that is already the live store."
                    .to_owned(),
            },
            Self::Committed => Recovery::Refuse {
                reason: "the migration finished. There is nothing to resume, and re-running any \
                         step would act on a store that is already the new one."
                    .to_owned(),
            },
        }
    }
}

/// Which of the three directories exist when a switch-over is interrupted.
///
/// The switch is two renames, and calling that pair "atomic" is exactly the
/// claim this type refuses to make: between them the disk shows a combination
/// that has to be read on its own terms. All eight are enumerated, and the ones
/// that do not determine what happened are refused rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchState {
    /// A directory sits at the source's name.
    pub source: bool,
    /// A directory sits at the archive's name.
    pub archive: bool,
    /// A directory sits at the destination's name.
    pub destination: bool,
}

const SWITCH_NOTHING: &str =
    "neither the source, the archive nor the destination exists. Whatever happened here is not      recoverable from the filesystem, and inventing a starting point would be inventing data.";
const SWITCH_ORPHAN_DESTINATION: &str =
    "only the destination exists. The source is gone and so is the archive, so nothing on disk      says whether the destination is a completed migration or an abandoned one. Renaming it into      place would be a guess.";
const SWITCH_ARCHIVE_ONLY: &str =
    "move the archive back to the source name. It is the only copy of the data, and no      destination was ever put in its place.";
const SWITCH_MID_RENAME: &str =
    "move the archive back to the source name, leaving the destination where it is. The switch      stopped between its two renames; the source is intact under the archive name and is still      the authority.";
const SWITCH_UNTOUCHED: &str =
    "only the source exists, exactly as before any migration. Start over from the beginning;      nothing needs undoing.";
const SWITCH_BUILT_NOT_SWITCHED: &str =
    "the source is in place and a destination exists beside it. Nothing has been moved, so the      destination can be validated against the source before anything is.";
const SWITCH_TWO_AUTHORITIES: &str =
    "a source and an archive both exist and there is no destination. Two directories claim to      hold the data and nothing distinguishes a half-finished restore from a half-finished      archive. Deleting or renaming either would destroy the one that turns out to be current.";
const SWITCH_IMPOSSIBLE: &str =
    "the source, the archive and the destination all exist. No sequence of this migration      produces all three at once, so the directory has been touched by something else. Nothing      here may be removed or renamed automatically.";

impl SwitchState {
    /// Every combination of the three, so a test can be exhaustive by
    /// construction rather than by a list someone maintained by hand.
    #[must_use]
    pub fn all() -> Vec<Self> {
        let mut out = Vec::with_capacity(8);
        for source in [false, true] {
            for archive in [false, true] {
                for destination in [false, true] {
                    out.push(Self {
                        source,
                        archive,
                        destination,
                    });
                }
            }
        }
        out
    }

    /// What to do, given only what is on disk.
    ///
    /// Reads. It does not move, rename or delete anything — deciding and acting
    /// are separated precisely so that a wrong decision cannot already have
    /// destroyed the evidence.
    #[must_use]
    pub fn recovery(self) -> Recovery {
        let refuse = |reason: &str| Recovery::Refuse {
            reason: reason.to_owned(),
        };
        let restore = |action: &str| Recovery::Restore {
            action: action.to_owned(),
        };
        let go = |next: Phase, rationale: &str| Recovery::Continue {
            next,
            rationale: rationale.to_owned(),
        };
        match (self.source, self.archive, self.destination) {
            (false, false, false) => refuse(SWITCH_NOTHING),
            (false, false, true) => refuse(SWITCH_ORPHAN_DESTINATION),
            (false, true, false) => restore(SWITCH_ARCHIVE_ONLY),
            (false, true, true) => restore(SWITCH_MID_RENAME),
            (true, false, false) => go(Phase::Prepared, SWITCH_UNTOUCHED),
            (true, false, true) => go(Phase::DestinationValidated, SWITCH_BUILT_NOT_SWITCHED),
            (true, true, false) => refuse(SWITCH_TWO_AUTHORITIES),
            (true, true, true) => refuse(SWITCH_IMPOSSIBLE),
        }
    }
}
