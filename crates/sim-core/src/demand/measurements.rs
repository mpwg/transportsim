use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{JourneyMode, ZoneId};

type Measurements<T> = BTreeMap<ZoneId, BTreeMap<ZoneId, BTreeMap<JourneyMode, T>>>;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct TravelWindows {
    pending: Measurements<Sample>,
    estimates: Measurements<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Sample {
    count: u64,
    #[serde(with = "decimal")]
    total: u128,
}

impl TravelWindows {
    pub(super) fn close(&mut self) {
        for (from, destinations) in std::mem::take(&mut self.pending) {
            let estimates = self.estimates.entry(from).or_default();
            for (to, modes) in destinations {
                let by_mode = estimates.entry(to).or_default();
                for (mode, sample) in modes {
                    by_mode.insert(mode, (sample.total / u128::from(sample.count)) as u64);
                }
            }
        }
    }

    pub(super) fn record(
        &mut self,
        from: ZoneId,
        to: ZoneId,
        mode: JourneyMode,
        ticks: u64,
        count: u64,
    ) {
        let sample = self
            .pending
            .entry(from)
            .or_default()
            .entry(to)
            .or_default()
            .entry(mode)
            .or_default();
        sample.count += count;
        sample.total += u128::from(count) * u128::from(ticks);
    }

    pub(super) fn estimate(&self, from: ZoneId, to: ZoneId, mode: JourneyMode) -> Option<u64> {
        self.estimates.get(&from)?.get(&to)?.get(&mode).copied()
    }

    pub(super) fn valid(&self, zone_exists: impl Fn(ZoneId) -> bool, arrived: u64) -> bool {
        let mut count = 0u128;
        for (&from, destinations) in &self.pending {
            if !zone_exists(from) {
                return false;
            }
            for (&to, samples) in destinations {
                if !zone_exists(to) {
                    return false;
                }
                for sample in samples.values() {
                    if sample.count == 0
                        || sample.total > u128::from(sample.count) * u128::from(u64::MAX)
                    {
                        return false;
                    }
                    count += u128::from(sample.count);
                }
            }
        }
        count <= u128::from(arrived)
            && self.estimates.iter().all(|(&from, destinations)| {
                zone_exists(from) && destinations.keys().copied().all(&zone_exists)
            })
    }
}

// Canonical JSON represents totals wider than u64 as decimal strings.
pub(super) mod decimal {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<u128, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}
