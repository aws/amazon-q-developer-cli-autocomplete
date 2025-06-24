#![cfg(target_os = "macos")]
// Mega yikes on this.
#![allow(deprecated)]
#![allow(unsafe_op_in_unsafe_fn)]

pub mod accessibility;
pub mod applications;
pub mod bundle;
pub mod caret_position;
pub mod image;
pub mod os;
pub mod url;
mod util;
pub mod window_server;

pub use util::{
    NotificationCenter,
    get_user_info_from_notification,
};
pub use window_server::{
    WindowServer,
    WindowServerEvent,
};
