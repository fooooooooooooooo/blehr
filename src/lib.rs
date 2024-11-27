#![warn(clippy::all, future_incompatible, nonstandard_style, rust_2018_idioms)]

use std::collections::HashSet;
use std::pin::Pin;

use btleplug::api::bleuuid::{uuid_from_u16, BleUuid};
use btleplug::api::{Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::{Stream, StreamExt};
use log::{debug, info};
use uuid::Uuid;

pub const HEARTRATE_CHARACTERISTIC_UUID: Uuid = uuid_from_u16(0x2A37);

#[derive(thiserror::Error, Debug)]
pub enum Error {
  /// The device did not have the wanted characteristic, or it could not be
  /// accessed
  #[error("The device did not have the wanted characteristic, or it could not be accessed")]
  CharacteristicNotFound,
  /// An error occurred in the underlying BLE library
  #[error("An error occurred in the underlying BLE library: {0}")]
  Blte(#[from] btleplug::Error),
  /// An error occurred while getting adapters
  #[error("An error occurred while getting adapters: {0}")]
  NoAdapters(btleplug::Error),
  /// No BLE adapters were found
  #[error("No BLE adapters were found")]
  NoAdapter,

  /// An error occurred while starting scan
  #[error("An error occurred while starting scan: {0}")]
  StartScan(btleplug::Error),

  /// An error occurred while stopping scan
  #[error("An error occurred while stopping scan: {0}")]
  StopScan(btleplug::Error),

  /// The peripheral was not found
  #[error("The peripheral was not found: {0}")]
  PeripheralNotFound(btleplug::Error),

  /// An error occurred while connecting to the peripheral
  #[error("An error occurred while connecting to the peripheral: {0}")]
  ConnectionFailed(btleplug::Error),

  /// An error occurred while discovering services
  #[error("An error occurred while discovering services: {0}")]
  ServiceDiscoveryFailed(btleplug::Error),

  /// An error occurred while subscribing to the characteristic
  #[error("An error occurred while subscribing to the characteristic: {0}")]
  SubscribeFailed(btleplug::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Provides an interface for scanning heart rate sensors.
pub struct Scanner {
  manager: Manager,
}

pub struct ScannerWithAdapter {
  _scanner: Scanner,
  adapter: Adapter,

  scanning: bool,

  incompatible_devices: HashSet<PeripheralId>,
}

impl Scanner {
  pub async fn new() -> Result<Self> {
    let manager = Manager::new().await?;

    Ok(Self { manager })
  }

  /// Starts scanning for heart rate sensors.
  pub async fn find_adapter(self) -> Result<ScannerWithAdapter> {
    let adapters = self.manager.adapters().await.map_err(Error::NoAdapters)?;
    let adapter = adapters.into_iter().nth(0).ok_or(Error::NoAdapter)?;

    let state = adapter
      .adapter_state()
      .await
      .expect("Adapter::adapter_state should not fail");
    debug!("Adapter state: {:?}", state);

    Ok(ScannerWithAdapter {
      _scanner: self,
      adapter,

      scanning: false,

      incompatible_devices: HashSet::new(),
    })
  }
}

impl ScannerWithAdapter {
  pub async fn start(&mut self) -> Result<()> {
    if !self.scanning {
      self
        .adapter
        .start_scan(ScanFilter::default())
        .await
        .map_err(Error::StartScan)?;
      self.scanning = true;
    }
    Ok(())
  }

  pub async fn stop(&mut self) -> Result<()> {
    if self.scanning {
      self.adapter.stop_scan().await.map_err(Error::StopScan)?;
      self.scanning = false;
    }
    Ok(())
  }

  pub async fn next_sensor(&mut self) -> Result<Option<Sensor>> {
    let mut events = self.adapter.events().await.expect("Adapter::events should not fail");

    while let Some(event) = events.next().await {
      if let CentralEvent::DeviceDiscovered(id) = event {
        if self.incompatible_devices.contains(&id) {
          continue;
        }

        let peripheral = self.adapter.peripheral(&id).await.map_err(Error::PeripheralNotFound)?;
        let properties = peripheral
          .properties()
          .await
          .expect("Peripheral::properties should not fail");
        let name = properties
          .and_then(|p| p.local_name)
          .map(|local_name| format!("Name: {local_name}"))
          .unwrap_or_default();

        debug!("DeviceDiscovered: {:?} {}", id, name);

        debug!("Connecting to peripheral: {:?}", id);

        peripheral.connect().await.map_err(Error::ConnectionFailed)?;

        debug!("Discovering services for peripheral: {:?}", id);

        peripheral
          .discover_services()
          .await
          .map_err(Error::ServiceDiscoveryFailed)?;

        debug!("Discovering characteristics for peripheral: {:?}", id);

        let characteristic = peripheral
          .characteristics()
          .iter()
          .find(|c| c.uuid == HEARTRATE_CHARACTERISTIC_UUID)
          .cloned();

        if let Some(characteristic) = characteristic {
          debug!("Found characteristic: {:?}", characteristic);
          return Ok(Some(Sensor::new(peripheral, characteristic)));
        } else {
          debug!("Characteristic not found");
          self.incompatible_devices.insert(id.clone());
        }

        debug!("Disconnecting from peripheral: {:?}", id);

        peripheral
          .disconnect()
          .await
          .expect("Peripheral::disconnect should not fail");
      }
    }

    Ok(None)
  }

  pub async fn read_events(&self) -> Result<()> {
    let mut events = self.adapter.events().await.expect("Adapter::events should not fail");

    while let Some(event) = events.next().await {
      match event {
        CentralEvent::DeviceDiscovered(id) => {
          let peripheral = self.adapter.peripheral(&id).await.map_err(Error::PeripheralNotFound)?;
          let properties = peripheral
            .properties()
            .await
            .expect("Peripheral::properties should not fail");
          let name = properties
            .and_then(|p| p.local_name)
            .map(|local_name| format!("Name: {local_name}"))
            .unwrap_or_default();

          info!("DeviceDiscovered: {:?} {}", id, name);
        }
        CentralEvent::DeviceUpdated(id) => info!("DeviceUpdated: {:?}", id),
        CentralEvent::DeviceConnected(id) => info!("DeviceConnected: {:?}", id),
        CentralEvent::DeviceDisconnected(id) => info!("DeviceDisconnected: {:?}", id),
        CentralEvent::ManufacturerDataAdvertisement { id, manufacturer_data } => {
          info!("ManufacturerDataAdvertisement: {:?} {:?}", id, manufacturer_data)
        }
        CentralEvent::ServiceDataAdvertisement { id, service_data } => {
          info!("ServiceDataAdvertisement: {:?} {:?}", id, service_data)
        }
        CentralEvent::ServicesAdvertisement { id, services } => {
          let services: Vec<String> = services.into_iter().map(|s| s.to_short_string()).collect();
          info!("ServicesAdvertisement: {:?}, {:?}", id, services);
        }
        CentralEvent::StateUpdate(state) => info!("StateUpdate: {:?}", state),
      }
    }

    Ok(())
  }
}

#[derive(Clone)]
pub struct Sensor {
  peripheral: Peripheral,
  characteristic: Characteristic,
}

impl Sensor {
  fn new(peripheral: Peripheral, characteristic: Characteristic) -> Self {
    Self {
      peripheral,
      characteristic,
    }
  }

  /// Provides a stream of heart rate values as [`Option`]\<u8>.
  /// If the heart rate value is 0, [`Option::None`] is returned.
  pub async fn hr_stream(&mut self) -> Result<Pin<Box<dyn Stream<Item = Option<u8>> + Send>>> {
    self
      .peripheral
      .subscribe(&self.characteristic)
      .await
      .map_err(Error::SubscribeFailed)?;

    let stream = self
      .peripheral
      .notifications()
      .await
      .expect("Peripheral::notifications should not fail");

    let uuid = self.characteristic.uuid;

    Ok(Box::pin(stream.filter_map(move |n| async move {
      if n.uuid == uuid {
        if n.value[1] == 0 {
          Some(None)
        } else {
          Some(Some(n.value[1]))
        }
      } else {
        None
      }
    })))
  }

  pub async fn name(&self) -> Option<String> {
    self.peripheral.properties().await.ok()?.and_then(|p| p.local_name)
  }
}
