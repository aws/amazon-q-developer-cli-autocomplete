#![allow(dead_code)]
#![allow(unused_variables)]

pub mod diagnostics;
mod env;
mod fs;
mod os;
mod sysinfo;

use std::sync::Arc;

pub use env::Env;
pub use fs::Fs;
pub use os::{
    Os,
    Platform,
};
pub use sysinfo::SysInfo;

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
        Self::default()
    }

    pub fn builder() -> ContextBuilder {
        ContextBuilder::new()
    }
}

impl Default for Context {
    fn default() -> Self {
        match cfg!(test) {
            true => Self {
                fs: Fs::new(),
                env: Env::new(),
                sysinfo: SysInfo::new(),
                platform: Platform::new(),
            },
            false => Self {
                fs: Default::default(),
                env: Default::default(),
                sysinfo: SysInfo::default(),
                platform: Platform::new(),
            },
        }
    }
}

#[derive(Default, Debug)]
pub struct ContextBuilder {
    fs: Option<Fs>,
    env: Option<Env>,
    sysinfo: Option<SysInfo>,
    platform: Option<Platform>,
}

pub const WINDOWS_USER_HOME: &str = "C:\\Users\\testuser";
pub const UNIX_USER_HOME: &str = "/home/testuser";

pub const ACTIVE_USER_HOME: &str = if cfg!(windows) {
    WINDOWS_USER_HOME
} else {
    UNIX_USER_HOME
};

impl ContextBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an immutable [Context] using real implementations for each field by default.
    pub fn build(self) -> Arc<Context> {
        let fs = self.fs.unwrap_or_default();
        let env = self.env.unwrap_or_default();
        let sysinfo = self.sysinfo.unwrap_or_default();
        let platform = self.platform.unwrap_or_default();
        Arc::new_cyclic(|_| Context {
            fs,
            env,
            sysinfo,
            platform,
        })
    }

    /// Builds an immutable [Context] using fake implementations for each field by default.
    pub fn build_fake(self) -> Context {
        let fs = self.fs.unwrap_or_default();
        let env = self.env.unwrap_or_default();
        let sysinfo = self.sysinfo.unwrap_or_default();
        let platform = self.platform.unwrap_or_default();
        Context {
            fs,
            env,
            sysinfo,
            platform,
        }
    }

    pub fn with_env(mut self, env: Env) -> Self {
        self.env = Some(env);
        self
    }

    pub fn with_fs(mut self, fs: Fs) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Creates a chroot filesystem and fake environment so that `$HOME`
    /// points to `<tempdir>/home/testuser`. Note that this replaces the
    /// [Fs] and [Env] currently set with the builder.
    #[cfg(test)]
    pub async fn with_test_home(mut self) -> Result<Self, std::io::Error> {
        let fs = Fs::new_chroot();
        fs.create_dir_all(ACTIVE_USER_HOME).await?;
        self.fs = Some(fs);

        if cfg!(windows) {
            self.env = Some(Env::from_slice(&[
                ("USERPROFILE", ACTIVE_USER_HOME),
                ("USERNAME", "testuser"),
            ]));
        } else {
            self.env = Some(Env::from_slice(&[("HOME", ACTIVE_USER_HOME), ("USER", "testuser")]));
        }

        Ok(self)
    }

    #[cfg(test)]
    pub fn with_env_var(mut self, key: &str, value: &str) -> Self {
        self.env = match self.env {
            Some(env) if cfg!(test) => {
                unsafe { env.set_var(key, value) };
                Some(env)
            },
            _ => Some(Env::from_slice(&[(key, value)])),
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_context_builder_with_test_home() {
        let ctx = ContextBuilder::new()
            .with_test_home()
            .await
            .unwrap()
            .with_env_var("hello", "world")
            .build();

        #[cfg(windows)]
        {
            assert!(ctx.fs.try_exists(WINDOWS_USER_HOME).await.unwrap());
            assert_eq!(ctx.env.get("USERPROFILE").unwrap(), WINDOWS_USER_HOME);
        }
        #[cfg(not(windows))]
        {
            assert!(ctx.fs.try_exists(UNIX_USER_HOME).await.unwrap());
            assert_eq!(ctx.env.get("HOME").unwrap(), UNIX_USER_HOME);
        }

        assert_eq!(ctx.env.get("hello").unwrap(), "world");
    }
}
