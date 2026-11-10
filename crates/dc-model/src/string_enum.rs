//! Helper for the format's open string enumerations.
//!
//! Shape types and endpoint styles are plain strings in the document. New ones
//! appear as Lucid ships new libraries, so every such enum keeps an `Other`
//! variant: an unrecognized value is preserved verbatim rather than dropped,
//! which is what lets a document using a library we have not implemented
//! survive a load-and-save cycle intact.

/// Generates an open string enum with serde support and an `Other` fallback.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($variant:ident => $text:literal),* $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)*
            /// A value this build does not recognize, kept as-is.
            Other(String),
        }

        impl $name {
            /// Every value this build knows, in declaration order.
            pub const KNOWN: &'static [&'static str] = &[$($text,)*];

            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $text,)*
                    Self::Other(s) => s.as_str(),
                }
            }

            /// Whether this value came from the published specification.
            pub fn is_known(&self) -> bool {
                !matches!(self, Self::Other(_))
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                match s {
                    $($text => Self::$variant,)*
                    other => Self::Other(other.to_string()),
                }
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                match s.as_str() {
                    $($text => Self::$variant,)*
                    _ => Self::Other(s),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok(Self::from(String::deserialize(d)?))
            }
        }
    };
}

pub(crate) use string_enum;
