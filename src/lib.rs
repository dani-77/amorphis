//! Security-critical primitives shared by the Amorphis backends.
//!
//! This crate does not yet expose a functional locker. Backend code must use
//! [`state::LockMachine`] as the only authority for entering the unlock path.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod auth;
pub mod backend;
pub mod background;
pub mod cli;
pub mod image;
mod indicator;
pub mod secret;
pub mod state;
pub mod system;
