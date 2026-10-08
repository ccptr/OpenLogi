//! Probing a receiver that is itself a HID++ 2.0 device: there are no pairing
//! registers and no arrival replay to ask, so the paired device's own answer
//! to a root ping is the only evidence that it is there.

use std::sync::Arc;

use hidpp::{
    channel::HidppChannel,
    feature::{CreatableFeature, root::RootFeature},
    receiver::hidpp20::{PAIRED_DEVICE_INDEX, Receiver as Hidpp20Receiver},
};
use openlogi_core::device::{DeviceInventory, DeviceKind, PairedDevice, ReceiverInfo};
use tokio::time::timeout;
use tracing::debug;

use super::unifying::unifying_probe_budget;
use super::{NodeProbe, PassContext, ProbeVerdict, RECEIVER_OPERATION_TIMEOUT};
use crate::backend::NodeInfo;
use crate::inventory::cache::{CacheKey, CacheOutcome, Cached, probe_or_reuse};
use crate::inventory::features::ProbedFeatures;
use crate::inventory::mappings::resolve_device_kind;

/// Probe a HID++ 2.0 receiver and the device behind it.
///
/// The receiver's unique id is the health gate: it is a feature call on the
/// receiver itself, so an answer proves the channel round-trips whether or
/// not the device is awake. A device that does not answer its ping is listed
/// offline from its last probe; one that was never seen cannot be listed at
/// all, because nothing on the receiver names it.
pub(super) async fn probe_hidpp20_receiver(
    channel: Arc<HidppChannel>,
    info: NodeInfo,
    receiver: Hidpp20Receiver,
    pass: PassContext<'_>,
) -> NodeProbe {
    let unique_id = match timeout(RECEIVER_OPERATION_TIMEOUT, receiver.get_unique_id()).await {
        Ok(Ok(uid)) => uid,
        Ok(Err(error)) => {
            debug!(?error, "receiver unique-id read failed");
            return NodeProbe::failed();
        }
        Err(_) => {
            debug!(
                budget = ?RECEIVER_OPERATION_TIMEOUT,
                "receiver unique-id read timed out"
            );
            return NodeProbe::failed();
        }
    };

    let slot = PAIRED_DEVICE_INDEX;
    // Same shape as a Unifying slot — a receiver plus a slot — so the two
    // share a key; the receivers' unique ids never collide across families.
    let id = CacheKey::UnifyingSlot {
        receiver_uid: unique_id.clone(),
        slot,
    };
    let cached = pass.cache.get(&id);
    let online = ping(&channel, slot).await;

    let budget = unifying_probe_budget(cached, pass.now, pass.timeouts);
    let probe_result = timeout(
        budget,
        probe_or_reuse(
            &channel,
            slot,
            Some(id.clone()),
            cached,
            online,
            pass.now,
            pass.subscriptions,
        ),
    )
    .await;
    let (probe, outcome) = if let Ok(result) = probe_result {
        result
    } else {
        debug!(
            slot,
            ?budget,
            "slot probe timed out; using cached data if available"
        );
        let probe = cached.map_or_else(ProbedFeatures::default, |entry| entry.probe.clone());
        (probe, CacheOutcome::Seen(id))
    };
    debug!(slot, online, name = ?probe.marketing_name, "HID++ 2.0 receiver slot");

    NodeProbe {
        inventory: Some(DeviceInventory {
            receiver: ReceiverInfo {
                name: crate::channel::route::receiver_display_name(info.product_id).to_string(),
                vendor_id: info.vendor_id,
                product_id: info.product_id,
                unique_id: Some(unique_id),
            },
            paired: paired_device(slot, probe, online, cached)
                .into_iter()
                .collect(),
        }),
        verdict: ProbeVerdict::Healthy { complete: true },
        outcomes: vec![outcome],
    }
}

/// Whether the device behind the receiver answers a root ping.
async fn ping(channel: &Arc<HidppChannel>, slot: u8) -> bool {
    let root = RootFeature::new(Arc::clone(channel), slot, 0);
    matches!(
        timeout(RECEIVER_OPERATION_TIMEOUT, root.ping(0x5a)).await,
        Ok(Ok(_))
    )
}

/// The slot's inventory entry, or `None` for a device that is neither
/// answering nor remembered: an entry with no identity would be a phantom.
fn paired_device(
    slot: u8,
    probe: ProbedFeatures,
    online: bool,
    cached: Option<&Cached>,
) -> Option<PairedDevice> {
    if !online && cached.is_none() {
        return None;
    }
    Some(PairedDevice {
        slot,
        codename: probe.marketing_name,
        // The wireless PID is only reported by HID++ 1.0 pairing registers.
        wpid: None,
        kind: resolve_device_kind(probe.kind, DeviceKind::Unknown),
        online,
        battery: probe.battery,
        model_info: probe.model_info,
        capabilities: probe.capabilities,
    })
}
