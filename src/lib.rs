#![doc = include_str!("../README.md")]
#![deny(unsafe_code)]
#![no_std]
extern crate alloc;

pub mod broadcast;
pub mod mpmc;
pub mod mpsc;
pub mod oneshot;
