//! # Gossip
//! This crate contains a gossip module of the voidphone Peer-to-Peer system.
//! It supports spreading knowledge in a peer-to-peer network.
//! In particular, it spreads contained knowledge to peers in the instance's view and to host internal modules which subscribed to the gossip module.
//! This document contains the final documentation of the gossip module. Further documentation can be found inside this rustdoc.
//! 
//! ## 1. Project Architecture
//! The project's architecture remains the same as in the [midterm report](https://gitlab.lrz.de/netintum/teaching/p2psec_projects_2022/Gossip-4/-/blob/main/docs/midterm-report.pdf).
//! 
//! ## 2. Software Documentation
//! 
//! This project is built, installed, and executed using rust's `cargo`.
//!
//! To build this project without executing it, execute:
//! ```bash
//! cargo build --release
//! ```
//! 
//! To run this project without installing it, execute:
//! ```bash
//! cargo run --release -- --config=PATH_TO_CONFIG
//! ```
//! 
//! To install and then execute this project, execute:
//! ```bash
//! cargo install --path=PATH_TO)BINARY
//! PATH_TO_BINARY --config=PATH_TO_CONFIG
//! ```
//! 
//! An RPS instance (e.g. the RPS Mockup) must be running for the gossip module to function.
//! 
//! ## 3. Future Work
//! The gossip module does not use the RPS module to sample peers. Instead, it samples peers itself from its view.
//! In future iterations, this functionality could be added. The basis for these functions are already included in the api and p2p servers.
//! 
//! Integration tests currently rely on manual verification. In future iterations, the results of the execution should be verified automatically.
//! 
//! ## 4. Workload Distribution
//! The workload distribution differs from the [initial report](https://gitlab.lrz.de/netintum/teaching/p2psec_projects_2022/Gossip-4/-/blob/main/docs/initial-report.pdf) with respect to the following points:
//! - Schambach built part of the Proof-of-Work module as well as a fragment of the broadcaster.
//! 
//! ## 5. Effort Spent for the Project
//! - Schambach: 65-75 hours
//! - Meixner: 65-75 hours

extern crate core;
pub mod broadcaster;
pub mod config;

mod common;
mod communication;
// mod pow;
mod publisher;
mod validator;
