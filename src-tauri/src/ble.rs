use async_trait::async_trait;
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
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
    /// Scan for `secs` and return named devices, strongest signal first.
    async fn discover(&self, secs: u64) -> Result<Vec<DiscoveredDevice>, String>;
    /// Current RSSI for one peripheral. `Ok(None)` means "not seen this round".
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
                if let Ok(Some(props)) = p.properties().await {
                    return Ok(props.rssi);
                }
            }
        }
        Ok(None)
    }
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
}
