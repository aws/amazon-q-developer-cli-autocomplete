#![allow(dead_code)]

pub mod diagnostics;
mod env;
mod fs;
mod sysinfo;

pub use env::Env;
pub use fs::Fs;
pub use sysinfo::SysInfo;

const WINDOWS_USER_HOME: &str = "C:\\Users\\testuser";
const UNIX_USER_HOME: &str = "/home/testuser";

pub const ACTIVE_USER_HOME: &str = if cfg!(windows) {
    WINDOWS_USER_HOME
} else {
    UNIX_USER_HOME
};

/// Struct that contains the interface to every system related IO operation.
///
/// Every operation that accesses the file system, environment, or other related platform
/// primitives should be done through a [Context] as this enables testing otherwise untestable
/// code paths in unit tests.
#[derive(Debug, Clone)]
pub struct Os {
    pub fs: Fs,
    pub env: Env,
    pub sysinfo: SysInfo,
}

impl Os {
    pub fn new() -> Self {
        Self {
            fs: Fs::new(),
            env: Env::new(),
            sysinfo: SysInfo::new(),
        }
    }

    /// TODO: delete this function
    #[cfg(test)]
    #[must_use]
    pub fn with_env_var(self, key: &str, value: &str) -> Self {
        unsafe { self.env.set_var(key, value) }
        self
    }
}

impl Default for Os {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_context_builder_with_test_home() {
        let os = Os::new().with_env_var("hello", "world");

        #[cfg(windows)]
        {
            assert!(os.fs.try_exists(ACTIVE_USER_HOME).await.unwrap());
            assert_eq!(os.env.get("USERPROFILE").unwrap(), ACTIVE_USER_HOME);
        }
        #[cfg(not(windows))]
        {
            assert!(os.fs.try_exists(ACTIVE_USER_HOME).await.unwrap());
            assert_eq!(os.env.get("HOME").unwrap(), ACTIVE_USER_HOME);
        }

        assert_eq!(os.env.get("hello").unwrap(), "world");
    }
}
