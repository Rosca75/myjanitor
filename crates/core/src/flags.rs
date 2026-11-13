//! A tiny in-house replacement for the `bitflags` crate.
//!
//! `flag_set!` declares a `Copy` newtype over an integer with named constants,
//! set operations, and serde support as an **array of snake_case names**
//! (unknown names are rejected), so the UI receives `["duplicate","lossy"]`.

macro_rules! flag_set {
    (
        $(#[$meta:meta])*
        pub struct $name:ident : $ty:ty {
            $( $(#[$fmeta:meta])* $flag:ident = $value:expr => $label:literal, )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
        pub struct $name($ty);

        #[allow(dead_code)]
        impl $name {
            $( $(#[$fmeta])* pub const $flag: $name = $name($value); )*

            /// Every flag with its wire name.
            pub const ALL: &'static [($name, &'static str)] = &[ $( ($name($value), $label), )* ];

            /// No flags.
            pub const fn empty() -> Self { $name(0) }
            /// Raw bits.
            pub const fn bits(self) -> $ty { self.0 }
            /// From raw bits, dropping unknown bits.
            pub fn from_bits_truncate(bits: $ty) -> Self {
                let mut all: $ty = 0;
                for (f, _) in Self::ALL { all |= f.0; }
                $name(bits & all)
            }
            /// True when no flag is set.
            pub const fn is_empty(self) -> bool { self.0 == 0 }
            /// True when every flag of `other` is set.
            pub const fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
            /// True when any flag of `other` is set.
            pub const fn intersects(self, other: Self) -> bool { self.0 & other.0 != 0 }
            /// Set the flags of `other`.
            pub fn insert(&mut self, other: Self) { self.0 |= other.0; }
            /// Clear the flags of `other`.
            pub fn remove(&mut self, other: Self) { self.0 &= !other.0; }
            /// Set or clear.
            pub fn set(&mut self, other: Self, on: bool) { if on { self.insert(other) } else { self.remove(other) } }
            /// Wire names of the set flags, in declaration order.
            pub fn names(self) -> Vec<&'static str> {
                Self::ALL.iter().filter(|(f, _)| self.contains(*f) && !f.is_empty()).map(|(_, n)| *n).collect()
            }
            /// Flag for a wire name.
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.iter().find(|(_, n)| *n == name).map(|(f, _)| *f)
            }
        }

        impl std::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self { $name(self.0 | rhs.0) }
        }
        impl std::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
        }
        impl std::ops::BitAnd for $name {
            type Output = Self;
            fn bitand(self, rhs: Self) -> Self { $name(self.0 & rhs.0) }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.names())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let names = self.names();
                let mut seq = s.serialize_seq(Some(names.len()))?;
                for n in names { seq.serialize_element(n)?; }
                seq.end()
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let names = Vec::<String>::deserialize(d)?;
                let mut out = $name(0);
                for n in names {
                    match $name::from_name(&n) {
                        Some(f) => out.insert(f),
                        None => return Err(serde::de::Error::custom(format!(
                            "unknown {} flag `{}`", stringify!($name), n))),
                    }
                }
                Ok(out)
            }
        }
    };
}

pub(crate) use flag_set;
