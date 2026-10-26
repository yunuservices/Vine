use std::fmt;

use super::generated::RELEASES;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub u32);

impl ProtocolVersion {
    pub const V1_8: Self = Self(47);
    pub const V1_11: Self = Self(315);
    pub const V1_12_2: Self = Self(340);
    pub const V1_13: Self = Self(393);
    pub const V1_16: Self = Self(735);
    pub const V1_19: Self = Self(759);
    pub const V1_19_1: Self = Self(760);
    pub const V1_19_3: Self = Self(761);
    pub const V1_20_2: Self = Self(764);
    pub const V1_20_3: Self = Self(765);
    pub const V1_20_5: Self = Self(766);

    pub fn from_handshake(raw: i32) -> Self {
        Self(raw.max(0) as u32)
    }

    pub fn is_known(self) -> bool {
        RELEASES.binary_search_by_key(&self.0, |(p, _)| *p).is_ok()
    }

    pub fn name(self) -> String {
        match RELEASES.binary_search_by_key(&self.0, |(p, _)| *p) {
            Ok(i) => RELEASES[i].1.to_string(),
            Err(_) => format!("unknown ({})", self.0),
        }
    }

    /// The newest release Vine supports
    pub fn latest() -> Self {
        Self(RELEASES[RELEASES.len() - 1].0)
    }

    /// The protocol of a release such as "1.8", "1.12.2" or "26.1"
    pub fn from_name(name: &str) -> Option<Self> {
        let wanted = release_number(name.trim())?;
        RELEASES
            .iter()
            .find(|(_, names)| {
                let (first, last) = names.split_once('-').unwrap_or((names, names));
                release_number(first).is_some_and(|first| first <= wanted)
                    && release_number(last).is_some_and(|last| wanted <= last)
            })
            .map(|(protocol, _)| Self(*protocol))
    }

    pub fn supported_range() -> String {
        let first = RELEASES[0].1.split('-').next().unwrap_or_default();
        let last = RELEASES[RELEASES.len() - 1]
            .1
            .rsplit('-')
            .next()
            .unwrap_or_default();
        format!("{first}-{last}")
    }
}

fn release_number(name: &str) -> Option<[u32; 3]> {
    let mut number = [0; 3];
    let mut parts = name.split('.');
    for (i, slot) in number.iter_mut().enumerate() {
        match parts.next() {
            Some(part) if part.bytes().all(|b| b.is_ascii_digit()) => *slot = part.parse().ok()?,
            None if i >= 2 => break,
            _ => return None,
        }
    }
    parts.next().is_none().then_some(number)
}

/// The client versions players may use; `None` leaves that end open
#[derive(Clone, Copy, Debug, Default)]
pub struct VersionRange {
    pub min: Option<ProtocolVersion>,
    pub max: Option<ProtocolVersion>,
}

impl VersionRange {
    pub fn contains(self, version: ProtocolVersion) -> bool {
        self.min.is_none_or(|min| version >= min) && self.max.is_none_or(|max| version <= max)
    }

    /// The end of the range nearest to `version`
    pub fn nearest(self, version: ProtocolVersion) -> Option<ProtocolVersion> {
        match (self.min, self.max) {
            (Some(min), _) if version < min => Some(min),
            (_, Some(max)) if version > max => Some(max),
            _ => None,
        }
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (protocol {})", self.name(), self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_versions() {
        assert!(ProtocolVersion(47).is_known());
        assert_eq!(ProtocolVersion(47).name(), "1.8-1.8.9");
        assert!(!ProtocolVersion(4).is_known());
        assert!(!ProtocolVersion(9999).is_known());
    }

    #[test]
    fn test_release_names_resolve_inside_shared_protocols() {
        let protocol = |name| ProtocolVersion::from_name(name).map(|v| v.0);
        assert_eq!(protocol("1.8"), Some(47));
        assert_eq!(protocol("1.8.9"), Some(47));
        assert_eq!(protocol("1.12.2"), Some(340));
        assert_eq!(protocol("1.20.3"), Some(765));
        assert_eq!(protocol("1.20.4"), Some(765));
        assert_eq!(protocol("1.21"), Some(767));
        assert_eq!(protocol("26.1.1"), Some(775));
        assert_eq!(protocol(" 26.2 "), Some(776));
        for unknown in [
            "", "1", "1.8.10", "1.20.7", "1.8.x", "1.8.", "+1.8", "1.8.9.1", "27.1",
        ] {
            assert_eq!(protocol(unknown), None, "{unknown}");
        }
    }
}
