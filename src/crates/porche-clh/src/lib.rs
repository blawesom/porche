//! porche-clh — Cloud Hypervisor API client (E-4: the host-agent drives CLH directly —
//! no containerd, no shim, no annotations) and the frozen-device VM configuration
//! (SEC-3/SEC-4, resource ceilings I-9).

pub mod client;
pub mod config;

pub use client::{ClhClient, ClhError};
pub use config::VmConfig;
