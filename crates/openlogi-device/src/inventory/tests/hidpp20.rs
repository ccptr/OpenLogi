//! The HID++ 2.0 receiver: its own feature table is the health gate, and the
//! device behind it is found by pinging its index.

use super::*;

/// The Lightspeed receiver verified with a PRO X3 SUPERSTRIKE.
const HIDPP20_RECEIVER_PID: u16 = 0xc54f;

/// The unit id [`receiver`] reports for itself.
const RECEIVER_UID: &str = "12345678";

/// A long HID++ 2.0 reply to `request` — the receiver answers short requests
/// with long reports too.
fn reply(request: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut report = vec![0x11, request[1], request[2], request[3]];
    report.extend_from_slice(payload);
    report.resize(20, 0);
    report
}

/// The receiver itself, as captured: `DeviceInformation` sits at feature
/// index 2 and reports the unit id.
fn receiver(request: &[u8]) -> Option<Vec<u8>> {
    let [_, 0xff, feature, function, id_hi, id_lo, ..] = *request else {
        return None;
    };
    match (feature, function >> 4) {
        (0x00, 0x0) if [id_hi, id_lo] == [0x00, 0x03] => Some(reply(request, &[0x02])),
        (0x00, 0x0) => Some(reply(request, &[0x00])),
        (0x02, 0x0) => Some(reply(
            request,
            &[0x07, 0x12, 0x34, 0x56, 0x78, 0x00, 0x08, 0xc5, 0x4f],
        )),
        _ => None,
    }
}

/// The receiver with an awake device behind it: the device answers its root
/// feature and exposes nothing else.
fn receiver_with_an_awake_device(request: &[u8]) -> Option<Vec<u8>> {
    let [_, 0x01, 0x00, function, _, _, ping, ..] = *request else {
        return receiver(request);
    };
    match function >> 4 {
        0x0 => Some(reply(request, &[0x00])),
        0x1 => Some(reply(request, &[0x04, 0x02, ping])),
        _ => None,
    }
}

fn receiver_node(tag: &str) -> NodeInfo {
    let mut info = scripted_node_info(&format!("{tag}-{}", std::process::id()));
    info.product_id = HIDPP20_RECEIVER_PID;
    info
}

async fn probe(
    responder: fn(&[u8]) -> Option<Vec<u8>>,
    cache: &HashMap<CacheKey, Cached>,
) -> NodeProbe {
    let (raw, _handle) = ScriptedRawHidChannel::with_responder(responder);
    let channel = scripted_channel(raw.presenting_as(HIDPP20_RECEIVER_PID)).await;
    let pass = PassContext {
        cache,
        now: Instant::now(),
        subscriptions: None,
        timeouts: &ProbeTimeouts::DEFAULT,
    };
    probe_one(receiver_node("hidpp20"), channel, pass).await
}

fn slot_key() -> CacheKey {
    CacheKey::UnifyingSlot {
        receiver_uid: RECEIVER_UID.to_string(),
        slot: 1,
    }
}

#[tokio::test]
async fn an_awake_device_is_listed_at_the_paired_index() {
    let probe = probe(receiver_with_an_awake_device, &HashMap::new()).await;

    assert_eq!(probe.verdict, ProbeVerdict::Healthy { complete: true });
    let inventory = probe
        .inventory
        .expect("an answering receiver has an inventory");
    assert_eq!(inventory.receiver.name, "Lightspeed Receiver");
    assert_eq!(inventory.receiver.unique_id.as_deref(), Some(RECEIVER_UID));
    let [device] = inventory.paired.as_slice() else {
        panic!("expected one device, got {:?}", inventory.paired);
    };
    assert!(device.online);
    assert_eq!(
        DeviceRoute::for_slot(&inventory, device.slot),
        Some(DeviceRoute::Unifying {
            receiver_uid: RECEIVER_UID.to_string(),
            slot: 1,
        })
    );
}

#[tokio::test]
async fn a_silent_device_never_seen_is_not_listed() {
    let probe = probe(receiver, &HashMap::new()).await;

    assert_eq!(
        probe.verdict,
        ProbeVerdict::Healthy { complete: true },
        "the receiver answered, so an empty list is authoritative"
    );
    let inventory = probe
        .inventory
        .expect("an answering receiver has an inventory");
    assert!(inventory.paired.is_empty(), "{:?}", inventory.paired);
}

#[tokio::test]
async fn a_silent_device_seen_before_is_listed_offline_from_its_last_probe() {
    let mut entry = cache_entry();
    entry.probe = ProbedFeatures {
        marketing_name: Some("PRO X3 SUPERSTRIKE".to_string()),
        ..probed(Some(model([1, 2, 3, 4], None)), false)
    };
    let cache = HashMap::from([(slot_key(), entry)]);

    let probe = probe(receiver, &cache).await;

    let inventory = probe
        .inventory
        .expect("an answering receiver has an inventory");
    let [device] = inventory.paired.as_slice() else {
        panic!("expected one device, got {:?}", inventory.paired);
    };
    assert!(!device.online);
    assert_eq!(device.codename.as_deref(), Some("PRO X3 SUPERSTRIKE"));
    assert_eq!(
        probe
            .outcomes
            .iter()
            .filter_map(CacheOutcome::key)
            .collect::<Vec<_>>(),
        [&slot_key()]
    );
}

#[tokio::test]
async fn a_silent_receiver_is_a_failed_probe() {
    let probe = probe(|_| None, &HashMap::new()).await;

    assert_eq!(probe.verdict, ProbeVerdict::Failed);
    assert!(probe.inventory.is_none());
}
