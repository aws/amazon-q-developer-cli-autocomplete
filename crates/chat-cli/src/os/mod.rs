#![allow(dead_code)]

pub mod diagnostics;
mod env;
mod fs;
mod sysinfo;

pub use env::Env;
use eyre::Result;
pub use fs::Fs;
pub use sysinfo::SysInfo;

use crate::database::Database;
use crate::telemetry::TelemetryThread;

const WINDOWS_USER_HOME: &str = "C:\\Users\\testuser";
const UNIX_USER_HOME: &str = "/home/testuser";

pub const ACTIVE_USER_HOME: &str = if cfg!(windows) {
    WINDOWS_USER_HOME
} else {
    UNIX_USER_HOME
};

// TODO OS SHOULD NOT BE CLONE

/// Struct that contains the interface to every system related IO operation.
///
/// Every operation that accesses the file system, environment, or other related platform
/// primitives should be done through a [Context] as this enables testing otherwise untestable
/// code paths in unit tests.
#[derive(Clone, Debug)]
pub struct Os {
    pub fs: Fs,
    pub env: Env,
    pub sysinfo: SysInfo,
    pub database: Database,
    pub telemetry: TelemetryThread,
}

impl Os {
    pub async fn new() -> Result<Self> {
        let env = Env::new();
        let mut database = crate::database::Database::new().await?;
        let telemetry = crate::telemetry::TelemetryThread::new(&env, &mut database).await?;

        Ok(Self {
            fs: Fs::new(),
            env,
            sysinfo: SysInfo::new(),
            database,
            telemetry,
        })
    }

    /// TODO: delete this function
    #[cfg(test)]
    #[must_use]
    pub fn with_env_var(self, key: &str, value: &str) -> Self {
        unsafe { self.env.set_var(key, value) }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_context_builder_with_test_home() {
        let os = Os::new().await.unwrap().with_env_var("hello", "world");

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
