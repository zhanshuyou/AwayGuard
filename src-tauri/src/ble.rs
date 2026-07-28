use async_trait::async_trait;
use btleplug::api::{Central, Manager as _, Peripheral as _, PeripheralProperties, ScanFilter};
use btleplug::platform::{Adapter, Manager};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub rssi: Option<i16>,
}

#[async_trait]
pub trait ProximitySource: Send + Sync {
    /// Sleep `secs`, then return every named device accumulated in the adapter's
    /// scan cache since the scan was started (in `BleSource::new`), strongest
    /// cached signal first. This can include devices no longer in range and
    /// possibly-stale RSSI, since no new scan is started here.
    async fn discover(&self, secs: u64) -> Result<Vec<DiscoveredDevice>, String>;
    /// Current RSSI for one peripheral, matched by `id` only.
    ///
    /// - `Ok(Some(rssi))`: the peripheral is present and has a cached reading.
    /// - `Ok(None)`: the peripheral is absent from the scan cache (the real
    ///   "departed" signal), OR it is present but has no cached RSSI this
    ///   round (ambiguous in btleplug's API — also reported as "not seen").
    /// - `Err`: a sensor fault (e.g. `properties()` failed) while the
    ///   peripheral IS present. This is NOT evidence of departure and must
    ///   not be fed to the proximity state machine as a missing sample.
    async fn sample(&self, target_id: &str) -> Result<Option<i16>, String>;
}

pub struct BleSource {
    central: Adapter,
}

impl BleSource {
    pub async fn new() -> Result<Self, String> {
        let manager = Manager::new().await.map_err(|e| e.to_string())?;
        let central = manager
            .adapters()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or("no Bluetooth adapter found")?;
        central
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self { central })
    }
}

#[async_trait]
impl ProximitySource for BleSource {
    async fn discover(&self, secs: u64) -> Result<Vec<DiscoveredDevice>, String> {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let mut out = Vec::new();
        for p in self.central.peripherals().await.map_err(|e| e.to_string())? {
            if let Ok(Some(props)) = p.properties().await {
                // ~250 devices show up per scan; only named ones are pickable.
                if let Some(name) = props.local_name {
                    out.push(DiscoveredDevice { id: p.id().to_string(), name, rssi: props.rssi });
                }
            }
        }
        out.sort_by_key(|d| -d.rssi.unwrap_or(-127));
        Ok(out)
    }

    async fn sample(&self, target_id: &str) -> Result<Option<i16>, String> {
        for p in self.central.peripherals().await.map_err(|e| e.to_string())? {
            if p.id().to_string() == target_id {
                // Peripheral is present. resolve_sample() decides what a
                // properties() failure here means (a sensor fault, NOT
                // evidence of departure -- see its doc comment).
                let props = p.properties().await.map_err(|e| e.to_string());
                return resolve_sample(props);
            }
        }
        // Peripheral absent from the scan cache — the real "departed" signal.
        Ok(None)
    }
}

/// Pure decision logic for one `sample()` lookup, split out of `BleSource::sample`
/// so it is unit-testable without real Bluetooth hardware.
///
/// - `Err`: a sensor fault (e.g. `properties()` failed) while the peripheral
///   IS present. Propagated as-is -- this is NOT evidence of departure and
///   must not become `Ok(None)` (which the state machine reads as "not
///   seen"). This is the invariant LEDGER 1 exists to guarantee: an error
///   here must never silently look like a missing sample.
/// - `Ok(Some(props))`: peripheral present. `props.rssi` is `None` when it
///   has no cached RSSI this round, which is genuinely ambiguous in
///   btleplug's API and reported the same as "not seen".
/// - `Ok(None)`: not reachable from `BleSource::sample`'s call site (it only
///   calls this after finding the peripheral), included for completeness.
fn resolve_sample(properties: Result<Option<PeripheralProperties>, String>) -> Result<Option<i16>, String> {
    let props = properties?;
    Ok(props.and_then(|pr| pr.rssi))
}

/// Test double: replays a fixed sample sequence, repeating the last value.
pub struct FakeSource {
    samples: Mutex<std::collections::VecDeque<Option<i16>>>,
    last: Mutex<Option<i16>>,
}

impl FakeSource {
    pub fn new(samples: Vec<Option<i16>>) -> Self {
        Self { samples: Mutex::new(samples.into()), last: Mutex::new(None) }
    }
}

#[async_trait]
impl ProximitySource for FakeSource {
    async fn discover(&self, _secs: u64) -> Result<Vec<DiscoveredDevice>, String> {
        Ok(vec![DiscoveredDevice {
            id: "fake-device".into(),
            name: "Fake iPhone".into(),
            rssi: Some(-40),
        }])
    }

    async fn sample(&self, _target_id: &str) -> Result<Option<i16>, String> {
        let mut q = self.samples.lock().unwrap();
        match q.pop_front() {
            Some(v) => {
                *self.last.lock().unwrap() = v;
                Ok(v)
            }
            None => Ok(*self.last.lock().unwrap()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_source_replays_samples_in_order() {
        let src = FakeSource::new(vec![Some(-40), Some(-90), None]);
        assert_eq!(src.sample("any").await.unwrap(), Some(-40));
        assert_eq!(src.sample("any").await.unwrap(), Some(-90));
        assert_eq!(src.sample("any").await.unwrap(), None);
    }

    #[tokio::test]
    async fn fake_source_repeats_last_sample_when_exhausted() {
        let src = FakeSource::new(vec![Some(-50)]);
        assert_eq!(src.sample("any").await.unwrap(), Some(-50));
        assert_eq!(src.sample("any").await.unwrap(), Some(-50));
    }

    // LEDGER 1(a): resolve_sample is the pure core of BleSource::sample --
    // exercised here with no hardware and no mocking, since
    // PeripheralProperties derives Default.

    #[test]
    fn resolve_sample_propagates_a_sensor_fault_as_err() {
        // A properties() failure for a present peripheral is NOT evidence of
        // departure; it must come out as Err, never silently become Ok(None).
        assert_eq!(resolve_sample(Err("adapter reset".into())), Err("adapter reset".to_string()));
    }

    #[test]
    fn resolve_sample_extracts_rssi_when_present() {
        let props = PeripheralProperties { rssi: Some(-55), ..Default::default() };
        assert_eq!(resolve_sample(Ok(Some(props))), Ok(Some(-55)));
    }

    #[test]
    fn resolve_sample_is_none_when_present_but_no_cached_rssi() {
        let props = PeripheralProperties { rssi: None, ..Default::default() };
        assert_eq!(resolve_sample(Ok(Some(props))), Ok(None));
    }

    #[test]
    fn resolve_sample_is_none_when_peripheral_absent() {
        assert_eq!(resolve_sample(Ok(None)), Ok(None));
    }
}
