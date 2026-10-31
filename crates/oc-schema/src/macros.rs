// SPDX-License-Identifier: Apache-2.0
//! `string_enum!` — a TS string-literal union as a Rust enum.
//!
//! Every generated enum has an `Other(String)` fallback so values written by newer clients round-trip unchanged
//! (the TS model preserves them verbatim; zod only rejects the few unions it validates, which `validate` checks on
//! the raw JSON).

macro_rules! string_enum {
    ($(#[$m:meta])* $vis:vis enum $name:ident { $($(#[$vm:meta])* $var:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name {
            $($(#[$vm])* $var,)+
            /// A value this build does not know (preserved verbatim).
            Other(String),
        }

        impl $name {
            /// Known values in declaration order (the TS union order).
            pub const VARIANTS: &'static [$name] = &[$($name::$var),+];

            /// The JSON string.
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$var => $s,)+
                    $name::Other(s) => s.as_str(),
                }
            }

            /// Parses a JSON string (unknown values become `Other`).
            pub fn parse(s: &str) -> Self {
                match s {
                    $($s => $name::$var,)+
                    _ => $name::Other(s.to_string()),
                }
            }

            /// Whether this is one of the values this build knows.
            pub fn is_known(&self) -> bool {
                !matches!(self, $name::Other(_))
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = ::std::convert::Infallible;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok($name::parse(s))
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <::std::borrow::Cow<'de, str> as ::serde::Deserialize>::deserialize(d)?;
                Ok($name::parse(&s))
            }
        }
    };
}

pub(crate) use string_enum;
