use serde::{Deserialize, Serialize};

const DEMAND_DOMAIN: u64 = u64::from_be_bytes(*b"DEMANDV1");
const GROWTH_DOMAIN: u64 = u64::from_be_bytes(*b"GROWTHV1");

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(tag = "version", deny_unknown_fields)]
pub(crate) enum RandomState {
    #[serde(rename = "splitmix64_v1")]
    SplitMix64V1 {
        demand: SplitMix64,
        growth: SplitMix64,
    },
}

impl RandomState {
    pub(crate) fn seeded(seed: u64) -> Self {
        let mut demand = SplitMix64::seeded(seed, DEMAND_DOMAIN);
        // One fixed offset preserves the daily round-robin rotation.
        demand.next();
        Self::SplitMix64V1 {
            demand,
            growth: SplitMix64::seeded(seed, GROWTH_DOMAIN),
        }
    }

    pub(crate) fn demand_phase(self) -> u64 {
        match self {
            Self::SplitMix64V1 { demand, .. } => demand.value(),
        }
    }

    pub(crate) fn growth(&mut self) -> &mut SplitMix64 {
        match self {
            Self::SplitMix64V1 { growth, .. } => growth,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct SplitMix64(u64);

impl SplitMix64 {
    fn seeded(seed: u64, domain: u64) -> Self {
        Self(mix(seed ^ domain))
    }

    fn value(self) -> u64 {
        mix(self.0)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        self.value()
    }

    pub(crate) fn sample_up_to(&mut self, maximum: u32) -> u32 {
        if maximum == 0 {
            return 0;
        }
        (self.next() % (u64::from(maximum) + 1)) as u32
    }
}

fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
