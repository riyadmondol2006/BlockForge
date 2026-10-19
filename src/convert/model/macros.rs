// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Macros that stamp out the fieldless enums of the model (block state values, item stack enums, the generated
//! id enums). Every enum gets `ALL`, `NAMES` (the Java constant names), `ordinal`, `name`, `from_ordinal`,
//! `from_name`, and a `Debug` that prints the constant name.

/// Look up `name` in `names`: binary search through `sorted` (ordinals sorted by name) when it is not empty,
/// otherwise a linear scan.
pub(crate) fn find_name(names: &[&str], sorted: &[u16], name: &str) -> Option<usize> {
    if sorted.is_empty() {
        return names.iter().position(|n| *n == name);
    }
    sorted.binary_search_by(|&i| names[i as usize].cmp(name)).ok().map(|k| sorted[k] as usize)
}

macro_rules! named_enum {
    (
        $(#[$meta:meta])*
        enum $name:ident : $repr:ident { $($variant:ident),+ $(,)? }
        $(sorted [ $($sorted:expr),+ $(,)? ])?
    ) => {
        $(#[$meta])*
        #[repr($repr)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Number of constants.
            pub const COUNT: usize = [$(Self::$variant),+].len();
            /// Every constant in Java declaration order.
            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),+];
            /// The Java constant names, indexed by ordinal.
            pub const NAMES: [&'static str; Self::COUNT] = [$(stringify!($variant)),+];

            /// The position in the Java declaration order.
            pub const fn ordinal(self) -> usize {
                self as usize
            }

            /// The Java constant name.
            pub const fn name(self) -> &'static str {
                Self::NAMES[self as usize]
            }

            /// The constant at `ordinal`, if any.
            pub fn from_ordinal(ordinal: usize) -> Option<Self> {
                Self::ALL.get(ordinal).copied()
            }

            /// The constant with the Java name `name`.
            pub fn from_name(name: &str) -> Option<Self> {
                const SORTED: &[u16] = &[$($($sorted),+)?];
                $crate::convert::model::macros::find_name(&Self::NAMES, SORTED, name).map(|i| Self::ALL[i])
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(self.name())
            }
        }
    };
}

macro_rules! state_enum {
    (
        $(#[$meta:meta])*
        enum $name:ident : $repr:ident { $($variant:ident),+ $(,)? }
    ) => {
        named_enum! {
            $(#[$meta])*
            enum $name: $repr { $($variant),+ }
        }

        impl $crate::convert::model::state::StateEnum for $name {
            const VALUES: &'static [Self] = &Self::ALL;
            const VALUE_NAMES: &'static [&'static str] = &Self::NAMES;

            fn value_ordinal(self) -> u8 {
                self as u8
            }
        }
    };
}
