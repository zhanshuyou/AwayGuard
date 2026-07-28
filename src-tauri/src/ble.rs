use async_trait::async_trait;
use btleplug::api::{
    Central, CentralEvent, Manager as _, Peripheral as _, PeripheralProperties, ScanFilter,
};
use btleplug::platform::{Adapter, Manager, PeripheralId};
use futures::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the target may go without producing a single advertisement before
/// `sample()` reports it as departed. Five missed 2-second polls.
///
/// This window is the whole reason `BleSource` keeps its own liveness clock.
/// btleplug's peripheral cache is not a liveness signal: on the CoreBluetooth
/// backend a peripheral is only ever evicted on `DeviceDisconnected`
/// (`corebluetooth/internal.rs:815`), which requires an explicit GATT
/// connection to have dropped -- and AwayGuard only ever scans, never
/// connects. `PeripheralProperties::rssi` is likewise write-only: it starts
/// `None` and is thereafter only assigned `Some(rssi)` when an advertisement
/// arrives, never reset and never expired. So a phone that vanishes abruptly
/// (elevator, fire door, Bluetooth off, dead battery) would otherwise leave
/// the adapter reporting its last-heard RSSI forever, and the app reporting
/// "near, armed, healthy" -- a silent fail-open.
pub const LIVENESS_WINDOW: Duration = Duration::from_secs(10);

/// Last moment each peripheral was heard advertising, keyed by
/// `PeripheralId::to_string()` -- the same string `discover()` hands the UI
/// and the config stores as `target_id`. The `PeripheralId` is kept alongside
/// so `sample()` can do an O(1) adapter lookup rather than walking the ~250
/// entry peripheral list on every poll.
type LivenessMap = HashMap<String, (PeripheralId, Instant)>;

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
    /// - `Ok(Some(rssi))`: the peripheral advertised within `LIVENESS_WINDOW`
    ///   and has a cached reading.
    /// - `Ok(None)`: the peripheral has not advertised within
    ///   `LIVENESS_WINDOW` (the real "departed" signal — this covers both
    ///   "never seen" and "seen, then went silent"), OR it is live but has no
    ///   cached RSSI this round (ambiguous in btleplug's API — also reported
    ///   as "not seen").
    /// - `Err`: a sensor fault (e.g. `properties()` failed) while the
    ///   peripheral IS live. This is NOT evidence of departure and must
    ///   not be fed to the proximity state machine as a missing sample.
    async fn sample(&self, target_id: &str) -> Result<Option<i16>, String>;
}

pub struct BleSource {
    central: Adapter,
    /// Written only by `pump`, read only by `sample`.
    liveness: Arc<Mutex<LivenessMap>>,
    window: Duration,
    /// The task consuming the adapter's `CentralEvent` stream. Held so it can
    /// be aborted when this source is dropped rather than leaking.
    pump: tokio::task::JoinHandle<()>,
}

impl Drop for BleSource {
    fn drop(&mut self) {
        self.pump.abort();
    }
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

        // Subscribe BEFORE start_scan, so no advertisement arriving between
        // the two calls is missed.
        let mut events = central.events().await.map_err(|e| e.to_string())?;
        let liveness: Arc<Mutex<LivenessMap>> = Arc::new(Mutex::new(HashMap::new()));

        let sink = liveness.clone();
        let pump = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if let Some(id) = liveness_evidence(&event) {
                    // btleplug publishes central events on a 16-slot tokio
                    // broadcast channel and silently drops anything a slow
                    // subscriber lags past, so this body must stay trivial:
                    // one clock read and one map insert, no `.await` inside
                    // the critical section (the guard must never be held
                    // across a suspension point).
                    let now = Instant::now();
                    let mut map = sink.lock().unwrap();
                    map.insert(id.to_string(), (id.clone(), now));
                }
            }
        });

        central
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self { central, liveness, window: LIVENESS_WINDOW, pump })
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
        // Snapshot under the lock and drop the guard before any `.await`.
        let entry = {
            let map = self.liveness.lock().unwrap();
            map.get(target_id).cloned()
        };
        // The two clock reads live here, at the edge; every decision below
        // is made by pure functions that take time as an argument.
        let last_seen = entry.as_ref().map(|(_, seen)| *seen);
        let now = Instant::now();

        resolve_liveness_sample(last_seen, now, self.window, || async move {
            // Only reached for a live target, so the entry is guaranteed.
            let (id, _) = entry.expect("a live target always has a liveness entry");
            // O(1) lookup by id -- no walk over the ~250 scanned peripherals.
            // A failure here means the adapter could not hand back a
            // peripheral it just told us was advertising: a sensor fault, not
            // a departure, so it propagates as Err.
            let peripheral = self.central.peripheral(&id).await.map_err(|e| e.to_string())?;
            peripheral.properties().await.map_err(|e| e.to_string())
        })
        .await
    }
}

/// The whole of `sample()`'s decision with the Bluetooth I/O lifted out, so
/// it can be driven end to end in tests with no hardware and no sleeping.
///
/// `read_properties` is lazy on purpose: a target that has gone stale is
/// reported departed without touching the adapter at all, which is both the
/// fast path and the guarantee that a stale target can never be rescued by a
/// frozen cached RSSI.
async fn resolve_liveness_sample<F, Fut>(
    last_seen: Option<Instant>,
    now: Instant,
    window: Duration,
    read_properties: F,
) -> Result<Option<i16>, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<PeripheralProperties>, String>>,
{
    let elapsed = last_seen.map(|seen| now.saturating_duration_since(seen));
    if is_stale(elapsed, window) {
        // Never seen, or gone silent for longer than the window. THIS is the
        // departed signal -- btleplug's own caches cannot produce one (see
        // LIVENESS_WINDOW).
        return Ok(None);
    }
    resolve_sample(read_properties().await)
}

/// The peripheral a central event is evidence of *liveness* for, or `None` if
/// the event says nothing about whether a peripheral is still advertising.
///
/// Verified against btleplug 0.12.0's CoreBluetooth backend: one
/// `centralManager:didDiscoverPeripheral:advertisementData:RSSI:` callback
/// (`corebluetooth/central_delegate.rs:416`) fans out into
/// `DeviceDiscovered` (first sighting) or `DeviceUpdated` (subsequent
/// sightings that carry a name), plus a `ManufacturerDataAdvertisement`,
/// `ServiceDataAdvertisement` and/or `ServicesAdvertisement` for whichever
/// AD structures the packet actually contained.
///
/// Deliberately excluded:
/// - `DeviceConnected` / `DeviceDisconnected` / `DeviceServicesModified`:
///   GATT-connection lifecycle. AwayGuard never connects, and a disconnect
///   is the opposite of liveness.
/// - `StateUpdate`: adapter-wide, carries no peripheral id.
///
/// `RssiUpdate` is included for correctness on other backends but is dead
/// weight here: on CoreBluetooth it is only emitted from `on_read_rssi`
/// (`corebluetooth/internal.rs:1220`), i.e. in response to an explicit
/// `read_rssi()` on a connected peripheral -- never from a scan
/// advertisement, despite what the variant's own doc comment claims.
fn liveness_evidence(event: &CentralEvent) -> Option<&PeripheralId> {
    match event {
        CentralEvent::DeviceDiscovered(id)
        | CentralEvent::DeviceUpdated(id)
        | CentralEvent::ManufacturerDataAdvertisement { id, .. }
        | CentralEvent::ServiceDataAdvertisement { id, .. }
        | CentralEvent::ServicesAdvertisement { id, .. }
        | CentralEvent::RssiUpdate { id, .. } => Some(id),
        CentralEvent::DeviceConnected(_)
        | CentralEvent::DeviceDisconnected(_)
        | CentralEvent::DeviceServicesModified(_)
        | CentralEvent::StateUpdate(_) => None,
    }
}

/// Pure staleness decision, split out of `BleSource::sample` so departure can
/// be unit-tested without hardware and without sleeping. Deliberately takes
/// pre-measured elapsed time rather than calling `Instant::now()` itself --
/// the same shape as `GraceTimer` in `monitor.rs`.
///
/// `None` means the target has never been heard from at all, which is stale
/// by definition. Reaching the window exactly counts as stale (matching
/// `GraceTimer::advance`'s `>= grace`): the boundary belongs to the
/// conservative side, since under-reporting departure is the fail-open this
/// whole mechanism exists to close.
fn is_stale(elapsed_since_last_seen: Option<Duration>, window: Duration) -> bool {
    match elapsed_since_last_seen {
        None => true,
        Some(elapsed) => elapsed >= window,
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

    // CRITICAL 1: liveness. btleplug never evicts a scanned peripheral and
    // never expires its cached RSSI, so `sample()` could not report departure
    // for a phone that had already been discovered. These exercise the
    // advertisement-driven liveness clock that replaces that missing signal.

    const WINDOW: Duration = Duration::from_secs(10);

    /// A properties reader that fails the test if the stale path ever calls
    /// it -- a departed target must be reported without touching the adapter.
    async fn must_not_read() -> Result<Option<PeripheralProperties>, String> {
        panic!("a stale target must be reported departed without reading properties");
    }

    fn props(rssi: i16) -> Result<Option<PeripheralProperties>, String> {
        Ok(Some(PeripheralProperties { rssi: Some(rssi), ..Default::default() }))
    }

    #[tokio::test]
    async fn live_peripheral_reports_its_rssi() {
        let base = Instant::now();
        let seen = base;
        let now = base + Duration::from_secs(4); // well inside the window
        let got = resolve_liveness_sample(Some(seen), now, WINDOW, || async { props(-55) }).await;
        assert_eq!(got, Ok(Some(-55)));
    }

    #[tokio::test]
    async fn peripheral_silent_past_the_window_is_reported_departed() {
        // The defect this whole mechanism exists to close: the phone vanishes
        // abruptly (elevator, Bluetooth off, dead battery) and stops
        // advertising. btleplug keeps serving its last-heard RSSI forever, so
        // only elapsed time can reveal the departure.
        let base = Instant::now();
        let now = base + Duration::from_secs(11);
        let got = resolve_liveness_sample(Some(base), now, WINDOW, must_not_read).await;
        assert_eq!(got, Ok(None), "a target silent past the window must be reported departed");
    }

    #[tokio::test]
    async fn never_seen_peripheral_is_reported_departed() {
        // At launch nothing has advertised yet. This is Ok(None), not Err --
        // the tracker treats it as Away from Unknown, which CRITICAL 2's
        // guard in run_once refuses to lock on.
        let got = resolve_liveness_sample(None, Instant::now(), WINDOW, must_not_read).await;
        assert_eq!(got, Ok(None));
    }

    #[tokio::test]
    async fn a_live_peripheral_with_a_broken_adapter_still_errors() {
        // Staleness is Ok(None) (departure); a sensor fault is Err. A live
        // target whose properties() fails must NOT be laundered into a
        // departure -- that would let a broken adapter lock the screen.
        let base = Instant::now();
        let now = base + Duration::from_secs(2);
        let got = resolve_liveness_sample(Some(base), now, WINDOW, || async {
            Err("adapter reset".to_string())
        })
        .await;
        assert_eq!(got, Err("adapter reset".to_string()));
    }

    #[tokio::test]
    async fn constant_rssi_still_goes_departed_once_the_clock_goes_stale() {
        // THE decisive case. Every reading is an identical, healthy -45 dBm
        // -- exactly what btleplug's frozen cache serves up after a phone has
        // stopped advertising. The old value-only logic saw a strong signal
        // forever and could never report departure; only the timestamp
        // distinguishes "still here" from "gone".
        let base = Instant::now();
        let frozen = || async { props(-45) };

        // Polls at +0s, +2s, ... +8s: advertisements are still arriving, so
        // last_seen keeps up with now and the strong RSSI is real.
        for tick in [0, 2, 4, 6, 8] {
            let now = base + Duration::from_secs(tick);
            assert_eq!(
                resolve_liveness_sample(Some(now), now, WINDOW, frozen).await,
                Ok(Some(-45)),
                "while advertisements keep arriving the RSSI must be reported"
            );
        }

        // Advertisements stop at +8s. The cache keeps returning -45, so the
        // only thing that changes from here is elapsed time.
        let last_seen = base + Duration::from_secs(8);
        for tick in [10, 12, 14, 16] {
            let now = base + Duration::from_secs(tick);
            let elapsed = now - last_seen;
            let got = resolve_liveness_sample(Some(last_seen), now, WINDOW, frozen).await;
            if elapsed < WINDOW {
                assert_eq!(got, Ok(Some(-45)), "inside the window the last reading still stands");
            } else {
                assert_eq!(
                    got,
                    Ok(None),
                    "an unchanging -45 dBm must still become a departure once the clock goes stale"
                );
            }
        }

        // And it must stay departed, not flap back on the frozen value.
        let now = base + Duration::from_secs(300);
        assert_eq!(
            resolve_liveness_sample(Some(last_seen), now, WINDOW, must_not_read).await,
            Ok(None)
        );
    }

    #[test]
    fn is_stale_treats_a_never_seen_target_as_stale() {
        assert!(is_stale(None, WINDOW));
    }

    #[test]
    fn is_stale_is_false_inside_the_window() {
        assert!(!is_stale(Some(Duration::from_secs(9)), WINDOW));
        assert!(!is_stale(Some(Duration::ZERO), WINDOW));
    }

    #[test]
    fn is_stale_at_exactly_the_window_boundary() {
        // The boundary belongs to the conservative side: reaching the window
        // counts as stale, matching GraceTimer::advance's `>= grace`.
        // 9.999s is live, 10.000s is departed.
        assert!(!is_stale(Some(Duration::from_millis(9_999)), WINDOW));
        assert!(is_stale(Some(WINDOW), WINDOW), "exactly at the window must count as departed");
        assert!(is_stale(Some(Duration::from_millis(10_001)), WINDOW));
    }

    #[test]
    fn liveness_window_covers_five_missed_polls() {
        // The 2s poll interval in lib.rs must fit five times over, so a
        // single dropped advertisement can never look like a departure.
        assert_eq!(LIVENESS_WINDOW, Duration::from_secs(10));
    }

    // liveness_evidence decides which btleplug events reset the clock.
    // Getting this set wrong is silent: too few and a present phone goes
    // stale (spurious lock), too many and a GATT event could keep a departed
    // phone alive forever.

    fn peripheral_id(byte: u8) -> PeripheralId {
        PeripheralId::from(uuid::Uuid::from_bytes([byte; 16]))
    }

    #[test]
    fn advertisement_events_are_liveness_evidence() {
        let id = peripheral_id(1);
        // Every event a scan advertisement can produce on the CoreBluetooth
        // backend, per centralManager:didDiscoverPeripheral:...
        let advertisement_events = vec![
            CentralEvent::DeviceDiscovered(id.clone()),
            CentralEvent::DeviceUpdated(id.clone()),
            CentralEvent::ManufacturerDataAdvertisement {
                id: id.clone(),
                manufacturer_data: HashMap::new(),
            },
            CentralEvent::ServiceDataAdvertisement {
                id: id.clone(),
                service_data: HashMap::new(),
            },
            CentralEvent::ServicesAdvertisement { id: id.clone(), services: Vec::new() },
            CentralEvent::RssiUpdate { id: id.clone(), rssi: -50 },
        ];
        for event in advertisement_events {
            assert_eq!(
                liveness_evidence(&event),
                Some(&id),
                "{event:?} must reset the liveness clock"
            );
        }
    }

    #[test]
    fn connection_lifecycle_events_are_not_liveness_evidence() {
        // AwayGuard never connects. A disconnect in particular is the
        // opposite of liveness and must never refresh the clock.
        let id = peripheral_id(2);
        for event in [
            CentralEvent::DeviceConnected(id.clone()),
            CentralEvent::DeviceDisconnected(id.clone()),
            CentralEvent::DeviceServicesModified(id.clone()),
        ] {
            assert_eq!(liveness_evidence(&event), None, "{event:?} must not reset the clock");
        }
    }
}
