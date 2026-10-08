use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub address: String,
    pub filename: String,
    pub format: String,
    pub size_bytes: u64,
    pub created_at: String,
    pub default_prim: String,
    pub prim_count: u64,
}

/// Validated once at the service boundary before it can become a storage path.
#[derive(Debug, Clone)]
pub(crate) struct ContentAddress(String);
impl ContentAddress {
    pub fn parse(value: String) -> Result<Self> {
        let hash = value
            .strip_prefix("sha256:")
            .ok_or_else(|| Error::BadRequest("invalid content address".into()))?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::BadRequest("invalid content address".into()));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn hash(&self) -> &str {
        &self.0[7..]
    }
}
impl fmt::Display for ContentAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Format {
    Usda,
    Usdc,
    Usdz,
}
impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usda => "usda",
            Self::Usdc => "usdc",
            Self::Usdz => "usdz",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "usda" => Ok(Self::Usda),
            "usdc" => Ok(Self::Usdc),
            "usdz" => Ok(Self::Usdz),
            _ => Err(Error::internal(
                "stored construct has an unsupported format",
            )),
        }
    }
}

pub(crate) struct UploadName {
    pub filename: String,
    pub extension: String,
}
impl UploadName {
    pub fn parse(filename: String) -> Result<Self> {
        if filename.is_empty()
            || filename.len() > 160
            || !filename
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        {
            return Err(Error::BadRequest(
                "filename must be 1..160 ASCII letters, digits, dots, underscores or hyphens"
                    .into(),
            ));
        }
        let extension = filename
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["usd", "usda", "usdc", "usdz"].contains(&extension.as_str()) {
            return Err(Error::BadRequest(
                "upload must be .usd, .usda, .usdc or .usdz".into(),
            ));
        }
        Ok(Self {
            filename,
            extension,
        })
    }
}

pub(crate) struct NewRecord {
    pub address: ContentAddress,
    pub filename: String,
    pub format: Format,
    pub size_bytes: u64,
    pub default_prim: String,
    pub prim_count: u64,
}
