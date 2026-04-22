#![doc = include_str!("../README.md")]
#![deny(unsafe_code)]
#![no_std]
extern crate alloc;

pub mod broadcast;
mod error;
pub mod mpmc;
pub mod mpsc;
pub mod oneshot;
pub mod spsc;
pub mod watch;

pub use error::{RecvError, SendError};
