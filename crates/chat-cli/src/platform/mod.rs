#![allow(dead_code)]

pub mod diagnostics;
mod env;
mod fs;
mod os;
mod sysinfo;

pub use env::Env;
pub use fs::Fs;
pub use os::{
    Os,
    Platform,
};
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
pub struct Context {
    pub fs: Fs,
    pub env: Env,
    pub sysinfo: SysInfo,
    pub platform: Platform,
}

impl Context {
    pub fn new() -> Self {
        if cfg!(test) {
            let env = match cfg!(windows) {
                true => Env::from_slice(&[("USERPROFILE", ACTIVE_USER_HOME), ("USERNAME", "testuser")]),
                false => Env::from_slice(&[("HOME", ACTIVE_USER_HOME), ("USER", "testuser")]),
            };

            Self {
                fs: Fs::new(),
                env,
                sysinfo: SysInfo::new(),
                platform: Platform::new(),
            }
        } else {
            Self {
                fs: Fs::new(),
                env: Env::new(),
                sysinfo: SysInfo::new(),
                platform: Platform::new(),
            }
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

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_context_builder_with_test_home() {
        let ctx = Context::new().with_env_var("hello", "world");

        #[cfg(windows)]
        {
            assert!(ctx.fs.try_exists(ACTIVE_USER_HOME).await.unwrap());
            assert_eq!(ctx.env.get("USERPROFILE").unwrap(), ACTIVE_USER_HOME);
        }
        #[cfg(not(windows))]
        {
            assert!(ctx.fs.try_exists(ACTIVE_USER_HOME).await.unwrap());
            assert_eq!(ctx.env.get("HOME").unwrap(), ACTIVE_USER_HOME);
        }

        assert_eq!(ctx.env.get("hello").unwrap(), "world");
    }
}
