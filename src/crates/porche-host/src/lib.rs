//! porche-host — the host-side implementation: the direct executor (netns,
//! LUKS2, CLH — argv built as data and unit-tested), the netns composition,
//! volume ops, and the command-spec runner. The PoC CLI (main.rs) and the
//! console-host (`porche-web`, E-28) both drive these.

pub mod cmdspec;
pub mod exec;
pub mod netns;
pub mod volume;
