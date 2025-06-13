use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AuthStrategy {
    SigV4,
    BearerToken,
    Auto,
}

impl Default for AuthStrategy {
    fn default() -> Self {
        Self::BearerToken
    }
}
