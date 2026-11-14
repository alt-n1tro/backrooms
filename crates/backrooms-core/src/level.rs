//! Level identity.
//!
//! A [`LevelId`] is the permanent identity of a level profile and is what
//! appears in addresses. It says nothing about where the level sits in the
//! vertical stack; that is the job of [`crate::zstack`].
//!
//! All 128 ids are reserved from the start. Building a new level means filling
//! in a reserved id, which never moves or renumbers anything that exists.

use core::fmt;

/// Number of levels in the stack.
pub const LEVEL_COUNT: usize = 128;

/// Identity of a level profile, `0..128`.
///
/// This is a "newtype": a one-field struct that exists so the compiler stops
/// you passing a depth, a count or any other `u8` where a level is expected.
/// The field is private, so the only way to make one is [`LevelId::new`],
/// which enforces the range.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LevelId(u8);

impl LevelId {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub const fn new(id: u8) -> Option<Self> {
        if (id as usize) < LEVEL_COUNT { Some(Self(id)) } else { None }
    }

    #[inline]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Which family the id belongs to. The ranges are fixed for good.
    pub const fn group(self) -> LevelGroup {
        match self.0 {
            0..=63 => LevelGroup::Yellow,
            64..=83 => LevelGroup::Lore,
            84..=95 => LevelGroup::Nature,
            _ => LevelGroup::Strange,
        }
    }
}

impl fmt::Display for LevelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LevelGroup {
    /// Ids 0-63: parameter variants of the yellow rooms.
    Yellow,
    /// Ids 64-83: free reinterpretations of well-known levels.
    Lore,
    /// Ids 84-95: outdoor-like levels whose sky is the level above.
    Nature,
    /// Ids 96-127: uncanny one-offs.
    Strange,
}

impl LevelGroup {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Yellow => "yellow",
            Self::Lore => "lore",
            Self::Nature => "nature",
            Self::Strange => "strange",
        }
    }
}

/// What a caller outside the generator needs to know about a level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LevelInfo {
    pub id: LevelId,
    pub group: LevelGroup,
    /// Display name for the HUD. Empty while the level is not built.
    pub name: &'static str,
    /// False for a reserved slot with no generator yet. Such a level has no
    /// addresses: asking for one of its chunks is an error, not an empty room.
    pub built: bool,
    /// Floor-to-ceiling height in millimetres (0 while not built).
    pub ceiling_mm: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_range() {
        assert!(LevelId::new(127).is_some());
        assert!(LevelId::new(128).is_none());
    }

    #[test]
    fn group_sizes_match_the_design() {
        let count = |g: LevelGroup| (0..128).filter(|&i| LevelId::new(i).unwrap().group() == g).count();
        assert_eq!(count(LevelGroup::Yellow), 64);
        assert_eq!(count(LevelGroup::Lore), 20);
        assert_eq!(count(LevelGroup::Nature), 12);
        assert_eq!(count(LevelGroup::Strange), 32);
    }
}
