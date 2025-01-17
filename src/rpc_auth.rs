//! This module contains types related to authentication for the RPC methods.
//!
//! These types are designed to be flexible to facilitate adding additional
//! authentication methods in the future.
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::config_models::data_directory::DataDirectory;
use crate::config_models::network::Network;

/// enumerates neptune-core RPC authentication token types
///
/// a [Token] is passed and authenticated with every RPC method call.
///
/// this is intended to be extensible with new variants in the future.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Token {
    Cookie(Cookie), //  [u8; 32]

                    // possible future types, eg
                    // Basic{user: String, pass: String},
}

impl Token {
    /// authenticate this token against known valid token data.
    ///
    /// `valid_tokens` should be an array containing one valid token of each
    /// [Token] variant.
    ///
    /// validation occurs against first valid token of same variant type as
    /// `self`.  any subsequent valid tokens of same type are ignored.
    ///
    /// panics if `valid_tokens` does not contain a variant matching `self`.
    pub(crate) fn auth(&self, valid_tokens: &[Self]) -> Result<(), error::AuthError> {
        // find first valid_token of same variant as self, panic if none.
        let valid_token = valid_tokens
            .iter()
            .filter(|v| std::mem::discriminant(self) == std::mem::discriminant(v))
            .next()
            .expect("caller must provide one valid token of each variant");

        match (self, valid_token) {
            (Self::Cookie(c), Self::Cookie(valid)) => c.auth(valid),
            // uncomment this line if another variant is added.
            // _ => unreachable!(),
        }
    }
}

impl From<Cookie> for Token {
    fn from(c: Cookie) -> Self {
        Self::Cookie(c)
    }
}

/// defines size of cookie byte array
type CookieBytes = [u8; 32];

/// represents an RPC authentication cookie
///
/// a cookie file is created each time neptune-core is started.
///
/// local (same-device) RPC clients with read access to the cookie
/// file can read it and provide the cookie as an auth [Token]
/// when calling RPC methods.
///
/// The cookie serves a couple purposes:
///   1. proves to neptune-core that the client is on the same device and
///      has read access for files written by neptune-core.
///   2. enables automated authentication without requiring user to
///      manually set a password somewhere.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cookie(CookieBytes);

impl From<CookieBytes> for Cookie {
    fn from(bytes: CookieBytes) -> Self {
        Self(bytes)
    }
}

impl Cookie {
    /// try loading cookie from a file
    pub fn try_load(data_dir: &DataDirectory) -> Result<Self, error::CookieFileError> {
        let mut cookie: CookieBytes = [0; 32];
        let path = Self::cookie_file_path(data_dir);
        let mut f = File::open(&path).map_err(|e| error::CookieFileError {
            path: path.clone(),
            error: e,
        })?;

        f.read_exact(&mut cookie)
            .map_err(|e| error::CookieFileError { path, error: e })?;

        Ok(Self(cookie))
    }

    /// try creating a new cookie file
    pub fn try_new(data_dir: &DataDirectory) -> Result<Self, error::CookieFileError> {
        let secret = Self::gen_secret();
        let path = Self::cookie_file_path(data_dir);
        let mut file = File::create(&path).map_err(|e| error::CookieFileError {
            path: path.clone(),
            error: e,
        })?;
        file.write_all(&secret)
            .map_err(|e| error::CookieFileError { path, error: e })?;
        Ok(Self(secret))
    }

    /// authenticate against a known valid cookie
    pub fn auth(&self, valid: &Self) -> Result<(), error::AuthError> {
        match self == valid {
            true => Ok(()),
            false => Err(error::AuthError::InvalidCookie),
        }
    }

    fn gen_secret() -> CookieBytes {
        rand::random()
    }

    /// get cookie file path
    pub fn cookie_file_path(data_dir: &DataDirectory) -> PathBuf {
        data_dir.rpc_cookie_file_path()
    }
}

/// provides a hint neptune-core client can use to automate authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieHint {
    pub data_directory: DataDirectory,
    pub network: Network,
}

pub mod error {

    use super::*;

    /// enumerates possible rpc authentication errors
    #[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
    #[non_exhaustive]
    pub enum AuthError {
        #[error("invalid authentication cookie")]
        InvalidCookie,
    }

    /// enumerates possible cookie load errors
    #[derive(Debug, thiserror::Error)]
    #[error("cookie file error: {}, path: {}", self.error, self.path.display())]
    pub struct CookieFileError {
        pub path: PathBuf,

        #[source]
        pub error: std::io::Error,
    }
}
