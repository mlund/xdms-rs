//! Pure-Rust unpacker for **DMS** (Disk Masher System) Amiga disk archives.
//!
//! DMS is the de-facto Amiga format for compressed copies of non-DOS disks
//! (games, demos). This crate decompresses a `.dms` archive into a raw **ADF**
//! disk image, which is what Amiga emulators consume.
//!
//! It is a clean-room port of the public-domain C tool *xDMS* by André Rodrigues
//! de la Rocha (maintained by Heikki Orsila). See the crate README for usage.
//!
//! The crate is `no_std` + `alloc` when built with `default-features = false`;
//! the default `std` feature adds the [`std::io`]-based API.
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]
// Foundational pieces (crc, bit reader, …) are built bottom-up before their
// consumers exist; this avoids transient dead-code noise mid-port. Removed once
// all modules are wired together (final gate).
#![allow(dead_code)]

extern crate alloc;

mod bitreader;
mod crc;
