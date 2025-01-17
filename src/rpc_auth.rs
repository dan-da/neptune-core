use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::config_models::data_directory::DataDirectory;
use crate::config_models::network::Network;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Token {
    Cookie(Cookie), //  [u8; 32]
}

impl Token {
    pub fn auth(&self, valid_cookie: &Cookie) -> Result<(), error::AuthError> {
        match self {
            Self::Cookie(c) => c.auth(valid_cookie),
        }
    }
}

impl From<Cookie> for Token {
    fn from(c: Cookie) -> Self {
        Self::Cookie(c)
    }
}

type CookieBytes = [u8; 32];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cookie(CookieBytes);

impl From<CookieBytes> for Cookie {
    fn from(bytes: CookieBytes) -> Self {
        Self(bytes)
    }
}

impl Cookie {
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

    pub fn auth(&self, valid: &Self) -> Result<(), error::AuthError> {
        match self == valid {
            true => Ok(()),
            false => Err(error::AuthError::InvalidCookie),
        }
    }

    fn gen_secret() -> CookieBytes {
        rand::random()
    }

    pub fn cookie_file_path(data_dir: &DataDirectory) -> PathBuf {
        data_dir.rpc_cookie_file_path()
    }
}

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
