#![allow(unused)]

pub mod clipx {
    include!(concat!(env!("OUT_DIR"), "/clipx.rs"));
}

pub use clipx::*;
