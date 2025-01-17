//! This module contains types related to authentication for the RPC methods.
//!
//! These types are designed to be flexible to facilitate adding additional
//! authentication methods in the future.
use std::path::PathBuf;

use rand::distributions::Alphanumeric;
use rand::distributions::DistString;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

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
            .find(|v| std::mem::discriminant(self) == std::mem::discriminant(v))
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Cookie(CookieBytes);

impl From<CookieBytes> for Cookie {
    fn from(bytes: CookieBytes) -> Self {
        Self(bytes)
    }
}

impl Cookie {
    /// try loading cookie from a file
    pub async fn try_load(data_dir: &DataDirectory) -> Result<Self, error::CookieFileError> {
        let mut cookie: CookieBytes = [0; 32];
        let path = Self::cookie_file_path(data_dir);
        let mut f = tokio::fs::File::open(&path)
            .await
            .map_err(|e| error::CookieFileError {
                path: path.clone(),
                error: e,
            })?;

        f.read_exact(&mut cookie)
            .await
            .map_err(|e| error::CookieFileError { path, error: e })?;

        Ok(Self(cookie))
    }

    /// try creating a new cookie file
    ///
    /// This will overwrite any existing cookie file.
    ///
    /// The overwrite is performed via rename, so should be an atomic operation
    /// on most filesystems.
    ///
    /// note: will create missing directories in path if necessary.
    pub async fn try_new(data_dir: &DataDirectory) -> Result<Self, error::CookieFileError> {
        let secret = Self::gen_secret();
        let path = Self::cookie_file_path(data_dir);
        let mut path_tmp = path.clone();

        let extension = Alphanumeric.sample_string(&mut rand::thread_rng(), 10);
        path_tmp.set_extension(extension);

        if let Some(parent_dir) = path.parent() {
            tokio::fs::create_dir_all(&parent_dir)
                .await
                .map_err(|e| error::CookieFileError {
                    path: path.clone(),
                    error: e,
                })?;
        }

        // open new temp file
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path_tmp)
            .await
            .map_err(|e| error::CookieFileError {
                path: path.clone(),
                error: e,
            })?;

        // write to temp file
        file.write_all(&secret)
            .await
            .map_err(|e| error::CookieFileError {
                path: path.clone(),
                error: e,
            })?;

        // rename temp file.  rename is an atomic operation in most filesystems.
        tokio::fs::rename(&path_tmp, &path)
            .await
            .map_err(|e| error::CookieFileError {
                path: path.clone(),
                error: e,
            })?;

        Ok(Self(secret))
    }

    // creates a cookie that exists in mem only, no .cookie file written to disk.
    #[cfg(test)]
    pub fn new_in_mem() -> Self {
        Self(Self::gen_secret())
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
        pub error: tokio::io::Error,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::tests::shared::unit_test_data_directory;

    mod token {
        use super::*;

        mod cookie {
            use super::*;

            /// test token authentication, cookie variant.
            ///
            /// tests:
            ///  1. Token::auth() succeeds for valid token
            ///  2. Token::auth() returns AuthError::InvalidCookie for invalid token
            #[tokio::test]
            pub async fn auth() -> anyhow::Result<()> {
                let data_dir = unit_test_data_directory(Network::Main)?;

                let valid_tokens: Vec<Token> = vec![Cookie::try_new(&data_dir).await?.into()];
                let valid_token_loaded: Token = Cookie::try_load(&data_dir).await?.into();
                let invalid_token: Token = Cookie::new_in_mem().into();

                // verify that auth fails for invalid token.
                let result = invalid_token.auth(&valid_tokens);
                assert!(matches!(result, Err(error::AuthError::InvalidCookie)));

                // verify that auth succeeds for valid cookie.
                assert!(valid_token_loaded.auth(&valid_tokens).is_ok());

                Ok(())
            }
        }
    }

    mod cookie {
        use std::collections::HashSet;

        use super::*;

        /// tests cookies are unique
        ///
        /// invokes Cookie::try_new() 50 times and stores in HashSet.
        ///
        /// tests:
        ///  1. Verify that HashSet contains 50 items.
        #[tokio::test]
        pub async fn try_new_unique() -> anyhow::Result<()> {
            let data_dir = unit_test_data_directory(Network::RegTest)?;
            const NUM_COOKIES: usize = 50;

            let mut set: HashSet<Cookie> = Default::default();

            for _ in 0..NUM_COOKIES {
                set.insert(Cookie::try_new(&data_dir).await?);
            }

            // verify there are 50 unique cookies
            assert_eq!(set.len(), NUM_COOKIES);

            Ok(())
        }

        /// test cookie authentication.
        ///
        /// exercises:
        ///  1. Cookie::try_new()
        ///  2. Cookie::try_load()
        ///
        /// tests:
        ///  1. Cookie::auth() succeeds for valid cookie
        ///  2. Cookie::auth() returns AuthError::InvalidCookei for invalid cookie
        #[tokio::test]
        pub async fn auth() -> anyhow::Result<()> {
            let data_dir = unit_test_data_directory(Network::Alpha)?;

            let valid_cookie = Cookie::try_new(&data_dir).await?;
            let valid_cookie_loaded = Cookie::try_load(&data_dir).await?;
            let invalid_cookie = Cookie::new_in_mem();

            assert_ne!(valid_cookie, invalid_cookie);

            // verify that auth fails for invalid cookie.
            let result = invalid_cookie.auth(&valid_cookie);
            assert!(matches!(result, Err(error::AuthError::InvalidCookie)));

            // verify that auth succeeds for valid cookie.
            assert!(valid_cookie_loaded.auth(&valid_cookie).is_ok());

            Ok(())
        }
    }
}
