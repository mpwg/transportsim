mod codec;
pub(crate) mod v1;

pub use codec::{LoadError, SaveError};

use crate::Simulation;

impl Simulation {
    /// Validates and starts a new simulation from scenario JSON.
    pub fn from_scenario_json(bytes: &[u8]) -> Result<Self, LoadError> {
        let scenario: v1::ScenarioV1 = codec::decode(bytes)?;
        scenario.into_simulation()
    }
}
