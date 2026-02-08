//! Filament Editor library — re-exports for integration tests.

mod actor;
mod document;
pub mod types;
mod yrs_crdt;

pub use actor::CoordinatorActor;
pub use yrs_crdt::{PeerAwareness, YrsCrdt};

#[cfg(test)]
mod tests {
    mod actors;
}
