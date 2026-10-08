//! Implements receivers that are themselves HID++ 2.0 devices.
//!
//! Newer Lightspeed receivers drop the HID++ 1.0 register set the Unifying
//! path is built on: every register access is answered with a HID++ 2.0
//! `InvalidFeatureIndex` error. The receiver instead exposes its own feature
//! table at [`RECEIVER_DEVICE_INDEX`] and relays its paired device at
//! [`PAIRED_DEVICE_INDEX`].
//!
//! There is no public specification for this receiver family. Everything
//! here was read off a `046d:c54f` receiver paired with a PRO X3 SUPERSTRIKE;
//! only standard features (`0x0000`, `0x0003`) are used, never the receiver's
//! undocumented ones.

use std::sync::Arc;

use openlogi_device_registry::receiver::{ReceiverProtocol, find_receiver};

use crate::{
    channel::HidppChannel,
    feature::{CreatableFeature, device_information::DeviceInformationFeature, root::RootFeature},
    receiver::{RECEIVER_DEVICE_INDEX, ReceiverError},
};

/// The index the receiver relays its paired device at.
///
/// Reverse-engineered: the verified receiver answers a root ping here with the
/// device's protocol version and stays silent on every other index.
pub const PAIRED_DEVICE_INDEX: u8 = 0x01;

/// Implements a receiver that is itself a HID++ 2.0 device.
#[derive(Clone)]
pub struct Receiver {
    chan: Arc<HidppChannel>,
}

impl Receiver {
    /// Tries to initialize a new [`Receiver`] from a raw HID++ channel.
    ///
    /// Returns [`ReceiverError::UnknownReceiver`] when the channel's VID/PID
    /// doesn't match any known HID++ 2.0 receiver.
    pub fn new(chan: Arc<HidppChannel>) -> Result<Self, ReceiverError> {
        if find_receiver(chan.vendor_id, chan.product_id)
            .is_none_or(|receiver| receiver.protocol != ReceiverProtocol::Hidpp20)
        {
            return Err(ReceiverError::UnknownReceiver);
        }

        Ok(Self { chan })
    }

    /// Provides the unique ID of the receiver: the unit ID its own
    /// `DeviceInformation` feature reports, as uppercase hex. The verified
    /// receiver reports an empty serial number, so the unit ID is the only
    /// per-unit identity it has.
    pub async fn get_unique_id(&self) -> Result<String, ReceiverError> {
        let root = RootFeature::new(Arc::clone(&self.chan), RECEIVER_DEVICE_INDEX, 0);
        let Some(information) = root.get_feature(DeviceInformationFeature::ID).await? else {
            return Err(ReceiverError::UnknownReceiver);
        };
        let feature = DeviceInformationFeature::new(
            Arc::clone(&self.chan),
            RECEIVER_DEVICE_INDEX,
            information.index,
        );
        Ok(format_unit_id(feature.get_device_info().await?.unit_id))
    }
}

fn format_unit_id(unit_id: [u8; 4]) -> String {
    format!("{:08X}", u32::from_be_bytes(unit_id))
}

#[cfg(test)]
mod tests {
    use super::format_unit_id;

    #[test]
    fn unit_id_keeps_its_leading_zeroes() {
        assert_eq!(format_unit_id([0x00, 0x1a, 0xb2, 0x0c]), "001AB20C");
    }
}
