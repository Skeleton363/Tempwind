//! Test tooling for x64wasm: the instruction oracle (record natively, replay
//! through the translator) and the spike programs run by the JS hosts.

pub mod host;
pub mod oracle;
pub mod spike;
