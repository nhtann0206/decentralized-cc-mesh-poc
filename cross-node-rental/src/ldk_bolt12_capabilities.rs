use crate::AppState;
use node_app_sdk_rust::*;

/// Return all capabilities provided by this app.
pub fn all_provided_capabilities() -> Vec<ProvidedCapability> {
    let mut caps = Vec::new();

    // core.lightning.* capabilities (20)
    caps.push(cap("core.lightning.node_id", "Get the node's Lightning public key", vec![
        CapabilityExample { label: "Get node ID".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.lightning.get_node_info", "Get comprehensive node information", vec![
        CapabilityExample { label: "Get node info".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.lightning.get_balances", "Get all balance information (on-chain, lightning, capacity)", vec![
        CapabilityExample { label: "Get balances".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.lightning.list_channels", "List all payment channels", vec![
        CapabilityExample { label: "List channels".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.lightning.open_channel", "Open a payment channel with a peer", vec![
        CapabilityExample { label: "Open channel".into(), request: serde_json::json!({
            "node_id": "02abc...", "address": "127.0.0.1:9735",
            "channel_amount_sats": 100000
        }) },
    ]));
    caps.push(cap("core.lightning.close_channel", "Cooperatively close a payment channel", vec![
        CapabilityExample { label: "Close channel".into(), request: serde_json::json!({
            "channel_id": "abc123...", "counterparty_node_id": "02abc..."
        }) },
    ]));
    caps.push(cap("core.lightning.list_peers", "List all connected peers", vec![
        CapabilityExample { label: "List peers".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.lightning.connect_peer", "Connect to a Lightning peer", vec![
        CapabilityExample { label: "Connect peer".into(), request: serde_json::json!({
            "node_id": "02abc...", "address": "127.0.0.1:9735"
        }) },
    ]));
    caps.push(cap("core.lightning.disconnect_peer", "Disconnect from a Lightning peer", vec![
        CapabilityExample { label: "Disconnect peer".into(), request: serde_json::json!({
            "node_id": "02abc..."
        }) },
    ]));
    caps.push(cap("core.lightning.create_invoice", "Create a BOLT11 Lightning invoice", vec![
        CapabilityExample { label: "Create invoice".into(), request: serde_json::json!({
            "amount_msat": 10000, "description": "Test payment"
        }) },
    ]));
    caps.push(cap("core.lightning.send_payment", "Send a Lightning payment using a BOLT11 invoice", vec![
        CapabilityExample { label: "Send payment".into(), request: serde_json::json!({
            "bolt11": "lnbc..."
        }) },
    ]));
    caps.push(cap("core.lightning.get_payment_status", "Get the status of a payment by hash", vec![
        CapabilityExample { label: "Check payment".into(), request: serde_json::json!({
            "payment_hash": "abc123..."
        }) },
    ]));
    caps.push(cap("core.lightning.list_payments", "List payment history with filters", vec![
        CapabilityExample { label: "List payments".into(), request: serde_json::json!({}) },
        CapabilityExample { label: "Filter payments".into(), request: serde_json::json!({
            "direction": "inbound", "status": "succeeded", "limit": 20
        }) },
    ]));
    caps.push(cap("core.lightning.new_onchain_address", "Generate a new on-chain Bitcoin address", vec![
        CapabilityExample { label: "New address".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.lightning.sign_message", "Sign a message with the node's private key", vec![
        CapabilityExample { label: "Sign message".into(), request: serde_json::json!({
            "message": "Hello, Lightning!"
        }) },
    ]));
    caps.push(cap("core.lightning.decode_invoice", "Decode a BOLT11 invoice to extract payment hash, amount, etc.", vec![
        CapabilityExample { label: "Decode invoice".into(), request: serde_json::json!({
            "invoice": "lnbc..."
        }) },
    ]));
    caps.push(cap("core.lightning.get_gossip_metadata", "Get all custom gossip metadata from peers", vec![
        CapabilityExample { label: "Get gossip".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.lightning.set_gossip_metadata", "Set this node's custom gossip metadata", vec![
        CapabilityExample { label: "Set metadata".into(), request: serde_json::json!({
            "metadata": {"features": [], "friendly_name": "My Node"}
        }) },
    ]));
    caps.push(cap("core.lightning.verify_signature", "Verify a signature against a message and public key", vec![
        CapabilityExample { label: "Verify signature".into(), request: serde_json::json!({
            "message": "base64_encoded_message",
            "signature": "hex_encoded_signature",
            "public_key": "02abc..."
        }) },
    ]));
    caps.push(cap("core.lightning.refresh_peers", "Disconnect and reconnect all peers (gossip propagation workaround)", vec![
        CapabilityExample { label: "Refresh peers".into(), request: serde_json::json!({}) },
    ]));

    // core.lightning.bolt12.* capabilities (2) — M4 cross-node VM billing
    caps.push(cap("core.lightning.create_bolt12_offer", "Create a BOLT12 offer for receiving payments (zero-amount, reusable)", vec![
        CapabilityExample { label: "Create VM rental offer".into(), request: serde_json::json!({
            "description": "VM rental: session abc, rate 100 sats/min",
            "expiry_secs": 86400
        }) },
    ]));
    caps.push(cap("core.lightning.pay_bolt12_offer", "Pay a BOLT12 offer (send sats to offer creator)", vec![
        CapabilityExample { label: "Pay per-minute VM rental".into(), request: serde_json::json!({
            "offer": "lno1...",
            "amount_msat": 100000,
            "payer_note": "minute:1"
        }) },
    ]));

    // core.l402.* capabilities (4)
    caps.push(cap("core.l402.create_challenge", "Create an L402 challenge (invoice + macaroon) for an endpoint", vec![
        CapabilityExample { label: "Create L402 challenge".into(), request: serde_json::json!({
            "path": "/messages/receive", "method": "POST"
        }) },
    ]));
    caps.push(cap("core.l402.verify_auth", "Verify an L402 authorization header", vec![
        CapabilityExample { label: "Verify L402 auth".into(), request: serde_json::json!({
            "authorization_header": "LSAT <macaroon>:<preimage>"
        }) },
    ]));
    caps.push(cap("core.l402.get_invoice_by_hash", "Look up an L402 invoice by payment hash", vec![
        CapabilityExample { label: "Get invoice".into(), request: serde_json::json!({
            "payment_hash": "abc123..."
        }) },
    ]));
    caps.push(cap("core.l402.cleanup_expired", "Mark expired invoices as expired", vec![
        CapabilityExample { label: "Cleanup".into(), request: serde_json::json!({}) },
    ]));

    // core.ldk.recovery.* capabilities (7)
    caps.push(cap("core.ldk.recovery.get_health", "Get health status of the LDK node", vec![
        CapabilityExample { label: "Health check".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.ldk.recovery.diagnostics", "Get detailed diagnostics for debugging", vec![
        CapabilityExample { label: "Diagnostics".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.ldk.recovery.force_close_channel", "Force-close a specific channel", vec![
        CapabilityExample { label: "Force close".into(), request: serde_json::json!({
            "channel_id": "abc123...", "counterparty_node_id": "02abc..."
        }) },
    ]));
    caps.push(cap("core.ldk.recovery.force_close_all", "Force-close all channels (emergency)", vec![
        CapabilityExample { label: "Force close all".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.ldk.recovery.reconnect_peers", "Reconnect to all known peers", vec![
        CapabilityExample { label: "Reconnect".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.ldk.recovery.check_panic", "Check if the LDK node is in a panic state", vec![
        CapabilityExample { label: "Check panic".into(), request: serde_json::json!({}) },
    ]));
    caps.push(cap("core.ldk.recovery.get_logs", "Get recent LDK node log entries", vec![
        CapabilityExample { label: "Get logs".into(), request: serde_json::json!({ "lines": 50 }) },
    ]));

    caps
}

fn cap(name: &str, desc: &str, examples: Vec<CapabilityExample>) -> ProvidedCapability {
    ProvidedCapability {
        name: name.into(),
        description: desc.into(),
        request_schema: None,
        response_schema: None,
        priority: 50,
        examples,
    }
}

/// Dispatch a capability request to the appropriate handler.
/// Returns (success, payload).
pub fn dispatch_capability(
    state: &AppState,
    capability: &str,
    payload: &serde_json::Value,
) -> (bool, serde_json::Value) {
    match capability {
        // ---- core.lightning.* ----
        "core.lightning.node_id" => handle_node_id(state),
        "core.lightning.get_node_info" => handle_get_node_info(state),
        "core.lightning.get_balances" => handle_get_balances(state),
        "core.lightning.list_channels" => handle_list_channels(state),
        "core.lightning.open_channel" => handle_open_channel(state, payload),
        "core.lightning.close_channel" => handle_close_channel(state, payload),
        "core.lightning.list_peers" => handle_list_peers(state),
        "core.lightning.connect_peer" => handle_connect_peer(state, payload),
        "core.lightning.disconnect_peer" => handle_disconnect_peer(state, payload),
        "core.lightning.create_invoice" => handle_create_invoice(state, payload),
        "core.lightning.send_payment" => handle_send_payment(state, payload),
        "core.lightning.get_payment_status" => handle_get_payment_status(state, payload),
        "core.lightning.list_payments" => handle_list_payments(state, payload),
        "core.lightning.new_onchain_address" => handle_new_onchain_address(state),
        "core.lightning.sign_message" => handle_sign_message(state, payload),
        "core.lightning.decode_invoice" => handle_decode_invoice(payload),
        "core.lightning.get_gossip_metadata" => handle_get_gossip_metadata(state),
        "core.lightning.set_gossip_metadata" => handle_set_gossip_metadata(state, payload),
        "core.lightning.verify_signature" => handle_verify_signature(state, payload),
        "core.lightning.refresh_peers" => handle_refresh_peers(state),

        // ---- core.lightning.bolt12.* ----
        "core.lightning.create_bolt12_offer" => handle_create_bolt12_offer(state, payload),
        "core.lightning.pay_bolt12_offer" => handle_pay_bolt12_offer(state, payload),

        // ---- core.l402.* ----
        "core.l402.create_challenge" => handle_l402_create_challenge(state, payload),
        "core.l402.verify_auth" => handle_l402_verify_auth(state, payload),
        "core.l402.get_invoice_by_hash" => handle_l402_get_invoice_by_hash(state, payload),
        "core.l402.cleanup_expired" => handle_l402_cleanup_expired(state),

        // ---- core.ldk.recovery.* ----
        "core.ldk.recovery.get_health" => crate::recovery::handle_get_health(state),
        "core.ldk.recovery.diagnostics" => crate::recovery::handle_diagnostics(state),
        "core.ldk.recovery.force_close_channel" => crate::recovery::handle_force_close_channel(state, payload),
        "core.ldk.recovery.force_close_all" => crate::recovery::handle_force_close_all(state),
        "core.ldk.recovery.reconnect_peers" => crate::recovery::handle_reconnect_peers(state),
        "core.ldk.recovery.check_panic" => crate::recovery::handle_check_panic(state),
        "core.ldk.recovery.get_logs" => crate::recovery::handle_get_logs(state, payload),

        _ => err(format!("unknown capability: {}", capability)),
    }
}

// ============================================================================
// core.lightning.* handlers
// ============================================================================

fn handle_node_id(state: &AppState) -> (bool, serde_json::Value) {
    let node_id = state.node_manager.node().node_id().to_string();
    ok(serde_json::json!({ "node_id": node_id }))
}

fn handle_get_node_info(state: &AppState) -> (bool, serde_json::Value) {
    let node = state.node_manager.node();
    let node_id = node.node_id().to_string();

    let listening_addresses: Vec<String> = node
        .listening_addresses()
        .unwrap_or_default()
        .iter()
        .map(|a| a.to_string())
        .collect();

    let channels = node.list_channels();
    let num_channels = channels.len();
    let num_usable_channels = channels.iter().filter(|ch| ch.is_usable).count();
    let local_balance_msat: u64 = channels.iter().map(|ch| ch.outbound_capacity_msat).sum();
    let num_peers = node.list_peers().len();

    // Preserve old API contract: NodeInfo { node_id, listening_addresses,
    // num_channels, num_usable_channels, local_balance_msat, num_peers }
    ok(serde_json::json!({
        "node_id": node_id,
        "listening_addresses": listening_addresses,
        "num_channels": num_channels,
        "num_usable_channels": num_usable_channels,
        "local_balance_msat": local_balance_msat,
        "num_peers": num_peers,
    }))
}

fn handle_get_balances(state: &AppState) -> (bool, serde_json::Value) {
    let node = state.node_manager.node();
    let balances = node.list_balances();

    // Calculate lightning spendable from channel outbound capacities
    let channels = node.list_channels();
    let lightning_spendable_sats: u64 = channels
        .iter()
        .filter(|ch| ch.is_usable)
        .map(|ch| ch.outbound_capacity_msat / 1000)
        .sum();

    let onchain_total = balances.total_onchain_balance_sats;
    let onchain_spendable = balances.spendable_onchain_balance_sats;
    let lightning_total = balances.total_lightning_balance_sats;

    // Preserve old API contract: Balances { onchain_spendable_sats, onchain_total_sats,
    // lightning_spendable_sats, lightning_total_sats, total_spendable_sats, total_sats }
    ok(serde_json::json!({
        "onchain_spendable_sats": onchain_spendable,
        "onchain_total_sats": onchain_total,
        "lightning_spendable_sats": lightning_spendable_sats,
        "lightning_total_sats": lightning_total,
        "total_spendable_sats": onchain_spendable + lightning_spendable_sats,
        "total_sats": onchain_total + lightning_total,
    }))
}

fn handle_list_channels(state: &AppState) -> (bool, serde_json::Value) {
    let node = state.node_manager.node();
    let channels = node.list_channels();

    // Preserve old API contract: returns flat ChannelInfo[] array (not wrapped)
    let channel_list: Vec<serde_json::Value> = channels
        .iter()
        .map(|ch| {
            // Format user_channel_id as hex string (u128 → 16 bytes → 32 hex chars)
            let user_channel_id = format!("{:032x}", ch.user_channel_id.0);
            serde_json::json!({
                "channel_id": ch.channel_id.to_string(),
                "counterparty_node_id": ch.counterparty_node_id.to_string(),
                "funding_txo": ch.funding_txo.map(|txo| format!("{}:{}", txo.txid, txo.vout)),
                "channel_value_sats": ch.channel_value_sats,
                "outbound_capacity_msat": ch.outbound_capacity_msat,
                "inbound_capacity_msat": ch.inbound_capacity_msat,
                "is_usable": ch.is_usable,
                "is_channel_ready": ch.is_channel_ready,
                "is_outbound": ch.is_outbound,
                "is_public": ch.is_announced,
                "user_channel_id": user_channel_id,
            })
        })
        .collect();

    ok(serde_json::json!(channel_list))
}

fn handle_open_channel(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let node_id_str = match payload.get("node_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'node_id'".into()),
    };
    let address = match payload.get("address").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'address'".into()),
    };
    let channel_amount_sats = match payload.get("channel_amount_sats").and_then(|v| v.as_u64()) {
        Some(v) => v,
        None => return err("missing required field 'channel_amount_sats'".into()),
    };
    let push_msat = payload
        .get("push_to_counterparty_msat")
        .and_then(|v| v.as_u64());
    let announce = payload
        .get("announce_channel")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let node = state.node_manager.node();

    let pubkey: ldk_node::bitcoin::secp256k1::PublicKey = match node_id_str.parse() {
        Ok(pk) => pk,
        Err(e) => return err(format!("invalid node_id: {}", e)),
    };

    let socket_addr: ldk_node::lightning::ln::msgs::SocketAddress = match address.parse() {
        Ok(a) => a,
        Err(e) => return err(format!("invalid address: {}", e)),
    };

    let result = if announce {
        node.open_announced_channel(pubkey, socket_addr, channel_amount_sats, push_msat, None)
    } else {
        node.open_channel(pubkey, socket_addr, channel_amount_sats, push_msat, None)
    };

    match result {
        Ok(user_channel_id) => ok(serde_json::json!({
            "user_channel_id": user_channel_id.0.to_string(),
            "status": "opening"
        })),
        Err(e) => err(format!("failed to open channel: {}", e)),
    }
}

fn handle_close_channel(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let channel_id_str = match payload.get("channel_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'channel_id'".into()),
    };
    let counterparty_str = match payload.get("counterparty_node_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'counterparty_node_id'".into()),
    };

    let node = state.node_manager.node();

    let user_channel_id: u128 = match channel_id_str.parse() {
        Ok(id) => id,
        Err(e) => return err(format!("invalid channel_id (expected UserChannelId u128): {}", e)),
    };
    let user_channel_id = ldk_node::UserChannelId(user_channel_id);

    let counterparty: ldk_node::bitcoin::secp256k1::PublicKey = match counterparty_str.parse() {
        Ok(pk) => pk,
        Err(e) => return err(format!("invalid counterparty_node_id: {}", e)),
    };

    match node.close_channel(&user_channel_id, counterparty) {
        Ok(()) => ok(serde_json::json!({ "status": "closing" })),
        Err(e) => err(format!("failed to close channel: {}", e)),
    }
}

fn handle_list_peers(state: &AppState) -> (bool, serde_json::Value) {
    let node = state.node_manager.node();
    let peers = node.list_peers();

    // Preserve old API contract: returns flat PeerInfo[] array (not wrapped)
    let peer_list: Vec<serde_json::Value> = peers
        .iter()
        .map(|p| {
            serde_json::json!({
                "node_id": p.node_id.to_string(),
                "address": p.address.to_string(),
                "is_persisted": p.is_persisted,
                "is_connected": p.is_connected,
            })
        })
        .collect();

    ok(serde_json::json!(peer_list))
}

fn handle_connect_peer(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let node_id_str = match payload.get("node_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'node_id'".into()),
    };
    let address = match payload.get("address").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'address'".into()),
    };
    let persist = payload
        .get("persist")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let node = state.node_manager.node();

    let pubkey: ldk_node::bitcoin::secp256k1::PublicKey = match node_id_str.parse() {
        Ok(pk) => pk,
        Err(e) => return err(format!("invalid node_id: {}", e)),
    };

    let socket_addr: ldk_node::lightning::ln::msgs::SocketAddress = match address.parse() {
        Ok(a) => a,
        Err(e) => return err(format!("invalid address: {}", e)),
    };

    match node.connect(pubkey, socket_addr, persist) {
        Ok(()) => ok(serde_json::json!({ "status": "connected" })),
        Err(e) => err(format!("failed to connect peer: {}", e)),
    }
}

fn handle_disconnect_peer(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let node_id_str = match payload.get("node_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'node_id'".into()),
    };

    let node = state.node_manager.node();

    let pubkey: ldk_node::bitcoin::secp256k1::PublicKey = match node_id_str.parse() {
        Ok(pk) => pk,
        Err(e) => return err(format!("invalid node_id: {}", e)),
    };

    match node.disconnect(pubkey) {
        Ok(()) => ok(serde_json::json!({ "status": "disconnected" })),
        Err(e) => err(format!("failed to disconnect peer: {}", e)),
    }
}

fn handle_create_invoice(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let amount_msat = match payload.get("amount_msat").and_then(|v| v.as_u64()) {
        Some(v) => v,
        None => return err("missing required field 'amount_msat'".into()),
    };
    let description = match payload.get("description").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'description'".into()),
    };
    let expiry_secs = payload
        .get("expiry_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(3600) as u32;

    let node = state.node_manager.node();

    use ldk_node::lightning_invoice::{Bolt11InvoiceDescription, Description};
    let desc = match Description::new(description.to_string()) {
        Ok(d) => d,
        Err(e) => return err(format!("invalid description: {}", e)),
    };

    match node
        .bolt11_payment()
        .receive(amount_msat, &Bolt11InvoiceDescription::Direct(desc), expiry_secs)
    {
        Ok(invoice) => {
            // Preserve old API contract: CreateInvoiceResponse { invoice: String }
            ok(serde_json::json!({
                "invoice": invoice.to_string(),
            }))
        }
        Err(e) => err(format!("failed to create invoice: {}", e)),
    }
}

fn handle_send_payment(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let bolt11_str = match payload.get("bolt11").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'bolt11'".into()),
    };

    let node = state.node_manager.node();

    let invoice: ldk_node::lightning_invoice::Bolt11Invoice = match bolt11_str.parse() {
        Ok(inv) => inv,
        Err(e) => return err(format!("invalid bolt11 invoice: {}", e)),
    };

    let payment_hash = hex::encode(invoice.payment_hash());

    match node.bolt11_payment().send(&invoice, None) {
        Ok(payment_id) => {
            // After send() returns Ok, the payment may still be in flight.
            // Poll payment list to extract preimage (needed for L402).
            let mut preimage: Option<String> = None;
            let max_attempts = 50; // 50 * 100ms = 5s max wait
            for _ in 0..max_attempts {
                let payments = node.list_payments();
                if let Some(payment) = payments.iter().find(|p| p.id == payment_id) {
                    match payment.status {
                        ldk_node::payment::PaymentStatus::Succeeded => {
                            if let ldk_node::payment::PaymentKind::Bolt11 { preimage: Some(pi), .. } = &payment.kind {
                                preimage = Some(pi.to_string());
                            }
                            break;
                        }
                        ldk_node::payment::PaymentStatus::Failed => {
                            return err("payment failed after sending".into());
                        }
                        ldk_node::payment::PaymentStatus::Pending => {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }

            ok(serde_json::json!({
                "status": if preimage.is_some() { "succeeded" } else { "pending" },
                "payment_hash": payment_hash,
                "preimage": preimage,
            }))
        }
        Err(e) => err(format!("failed to send payment: {}", e)),
    }
}

// ============================================================================
// core.lightning.bolt12.* handlers — M4 cross-node VM billing
// ============================================================================

/// Create a zero-amount BOLT12 offer for receiving recurring payments.
/// Bob calls this when a VM session starts — the offer is sent to Alice
/// who pays it every billing interval.
fn handle_create_bolt12_offer(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let description = payload
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("VM rental payment");
    let expiry_secs = payload
        .get("expiry_secs")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let node = state.node_manager.node();
    match node.bolt12_payment().receive_variable_amount(description, expiry_secs) {
        Ok(offer) => {
            ok(serde_json::json!({
                "offer": offer.to_string(),
            }))
        }
        Err(e) => err(format!("failed to create BOLT12 offer: {:?}", e)),
    }
}

/// Pay a BOLT12 offer with a specified amount. Alice calls this every
/// billing interval (default 60s) to pay Bob for VM usage.
///
/// `payer_note` carries billing metadata (e.g. "minute:42, session:abc")
/// for Bob's verification. The payment itself is cryptographically proven
/// via BOLT12's invoice chain — payer_note is informational only.
fn handle_pay_bolt12_offer(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let offer_str = match payload.get("offer").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'offer'".into()),
    };
    let amount_msat = match payload.get("amount_msat").and_then(|v| v.as_u64()) {
        Some(v) => v,
        None => return err("missing required field 'amount_msat'".into()),
    };
    let payer_note = payload
        .get("payer_note")
        .and_then(|v| v.as_str())
        .map(String::from);

    let node = state.node_manager.node();

    // Parse the offer string
    let offer: ldk_node::lightning::offers::offer::Offer = match offer_str.parse() {
        Ok(o) => o,
        Err(e) => return err(format!("invalid BOLT12 offer: {:?}", e)),
    };

    match node
        .bolt12_payment()
        .send_using_amount(&offer, amount_msat, None, payer_note)
    {
        Ok(payment_id) => {
            // Poll for completion (same pattern as BOLT11 send_payment)
            let mut status_str = "pending";
            let max_attempts = 50; // 5s max
            for _ in 0..max_attempts {
                let payments = node.list_payments();
                if let Some(payment) = payments.iter().find(|p| p.id == payment_id) {
                    match payment.status {
                        ldk_node::payment::PaymentStatus::Succeeded => {
                            status_str = "succeeded";
                            break;
                        }
                        ldk_node::payment::PaymentStatus::Failed => {
                            return err("BOLT12 payment failed".into());
                        }
                        ldk_node::payment::PaymentStatus::Pending => {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
            ok(serde_json::json!({
                "status": status_str,
                "payment_id": hex::encode(payment_id.0),
            }))
        }
        Err(e) => err(format!("failed to pay BOLT12 offer: {:?}", e)),
    }
}

fn handle_get_payment_status(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let payment_hash_str = match payload.get("payment_hash").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'payment_hash'".into()),
    };

    let node = state.node_manager.node();
    let payments = node.list_payments();

    // Search for payment by hash
    let found = payments.iter().find(|p| {
        let hash = match &p.kind {
            ldk_node::payment::PaymentKind::Bolt11 { hash, .. } => hash.to_string(),
            ldk_node::payment::PaymentKind::Bolt11Jit { hash, .. } => hash.to_string(),
            ldk_node::payment::PaymentKind::Bolt12Offer { hash, .. } => {
                hash.map(|h| h.to_string()).unwrap_or_default()
            }
            ldk_node::payment::PaymentKind::Bolt12Refund { hash, .. } => {
                hash.map(|h| h.to_string()).unwrap_or_default()
            }
            ldk_node::payment::PaymentKind::Spontaneous { hash, .. } => hash.to_string(),
            ldk_node::payment::PaymentKind::Onchain { .. } => String::new(),
        };
        hash == payment_hash_str
    });

    match found {
        Some(payment) => {
            let status = match payment.status {
                ldk_node::payment::PaymentStatus::Pending => "pending",
                ldk_node::payment::PaymentStatus::Succeeded => "succeeded",
                ldk_node::payment::PaymentStatus::Failed => "failed",
            };

            let preimage = match &payment.kind {
                ldk_node::payment::PaymentKind::Bolt11 { preimage, .. } => {
                    preimage.map(|p| p.to_string())
                }
                _ => None,
            };

            ok(serde_json::json!({
                "status": status,
                "preimage": preimage,
                "amount_msat": payment.amount_msat,
            }))
        }
        None => err("payment not found".into()),
    }
}

fn handle_list_payments(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let direction_filter = payload.get("direction").and_then(|v| v.as_str());
    let status_filter = payload.get("status").and_then(|v| v.as_str());
    let kind_filter = payload.get("kind").and_then(|v| v.as_str());
    let page_num = payload.get("page").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let page_size = payload.get("page_size").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let sort_order = payload.get("sort_order").and_then(|v| v.as_str()).unwrap_or("desc");

    let node = state.node_manager.node();
    let all_payments = node.list_payments();

    let mut filtered: Vec<&ldk_node::payment::PaymentDetails> = all_payments
        .iter()
        .filter(|p| {
            if let Some(dir) = direction_filter {
                let is_inbound = p.direction == ldk_node::payment::PaymentDirection::Inbound;
                match dir {
                    "inbound" | "Inbound" => {
                        if !is_inbound { return false; }
                    }
                    "outbound" | "Outbound" => {
                        if is_inbound { return false; }
                    }
                    _ => {}
                }
            }
            if let Some(st) = status_filter {
                let status = match p.status {
                    ldk_node::payment::PaymentStatus::Pending => "pending",
                    ldk_node::payment::PaymentStatus::Succeeded => "succeeded",
                    ldk_node::payment::PaymentStatus::Failed => "failed",
                };
                if !status.eq_ignore_ascii_case(st) { return false; }
            }
            if let Some(k) = kind_filter {
                let kind_str = match &p.kind {
                    ldk_node::payment::PaymentKind::Bolt11 { .. } => "bolt11",
                    ldk_node::payment::PaymentKind::Bolt11Jit { .. } => "bolt11_jit",
                    ldk_node::payment::PaymentKind::Bolt12Offer { .. } => "bolt12_offer",
                    ldk_node::payment::PaymentKind::Bolt12Refund { .. } => "bolt12_refund",
                    ldk_node::payment::PaymentKind::Spontaneous { .. } => "spontaneous",
                    ldk_node::payment::PaymentKind::Onchain { .. } => "onchain",
                };
                if !kind_str.eq_ignore_ascii_case(k) { return false; }
            }
            true
        })
        .collect();

    // Sort by latest_update_timestamp
    if sort_order == "asc" {
        filtered.sort_by_key(|p| p.latest_update_timestamp);
    } else {
        filtered.sort_by(|a, b| b.latest_update_timestamp.cmp(&a.latest_update_timestamp));
    }

    let total_items = filtered.len() as u32;
    let total_pages = if page_size > 0 { total_items.div_ceil(page_size as u32) } else { 1 };
    let offset = (page_num.saturating_sub(1)) * page_size;

    let page: Vec<serde_json::Value> = filtered
        .into_iter()
        .skip(offset)
        .take(page_size)
        .map(|p| {
            let (kind_str, _payment_hash) = match &p.kind {
                ldk_node::payment::PaymentKind::Bolt11 { hash, .. } => ("bolt11", hash.to_string()),
                ldk_node::payment::PaymentKind::Bolt11Jit { hash, .. } => ("bolt11_jit", hash.to_string()),
                ldk_node::payment::PaymentKind::Bolt12Offer { hash, .. } => {
                    ("bolt12_offer", hash.map(|h| h.to_string()).unwrap_or_default())
                }
                ldk_node::payment::PaymentKind::Bolt12Refund { hash, .. } => {
                    ("bolt12_refund", hash.map(|h| h.to_string()).unwrap_or_default())
                }
                ldk_node::payment::PaymentKind::Spontaneous { hash, .. } => ("spontaneous", hash.to_string()),
                ldk_node::payment::PaymentKind::Onchain { txid, .. } => ("onchain", txid.to_string()),
            };
            let preimage = match &p.kind {
                ldk_node::payment::PaymentKind::Bolt11 { preimage, .. } => preimage.map(|pi| pi.to_string()),
                _ => None,
            };
            let fee_paid_msat = p.fee_paid_msat;
            let status = match p.status {
                ldk_node::payment::PaymentStatus::Pending => "pending",
                ldk_node::payment::PaymentStatus::Succeeded => "succeeded",
                ldk_node::payment::PaymentStatus::Failed => "failed",
            };
            let direction = match p.direction {
                ldk_node::payment::PaymentDirection::Inbound => "inbound",
                ldk_node::payment::PaymentDirection::Outbound => "outbound",
            };

            // Preserve old API contract: PaymentDetailsResponse fields
            serde_json::json!({
                "id": p.id.to_string(),
                "kind": kind_str,
                "preimage": preimage,
                "amount_msat": p.amount_msat,
                "fee_paid_msat": fee_paid_msat,
                "direction": direction,
                "status": status,
                "latest_update_timestamp": p.latest_update_timestamp,
            })
        })
        .collect();

    // Preserve old API contract: ListPaymentsResponse { payments, pagination }
    ok(serde_json::json!({
        "payments": page,
        "pagination": {
            "current_page": page_num,
            "page_size": page_size,
            "total_items": total_items,
            "total_pages": total_pages,
            "has_next": (page_num as u32) < total_pages,
            "has_previous": page_num > 1,
        },
    }))
}

fn handle_new_onchain_address(state: &AppState) -> (bool, serde_json::Value) {
    let node = state.node_manager.node();
    match node.onchain_payment().new_address() {
        Ok(address) => ok(serde_json::json!({ "address": address.to_string() })),
        Err(e) => err(format!("failed to generate address: {}", e)),
    }
}

fn handle_sign_message(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let message = match payload.get("message").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'message'".into()),
    };

    let node = state.node_manager.node();
    let signature = node.sign_message(message.as_bytes());
    ok(serde_json::json!({ "signature": signature }))
}

fn handle_decode_invoice(payload: &serde_json::Value) -> (bool, serde_json::Value) {
    use std::str::FromStr;

    let invoice_str = match payload.get("invoice").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'invoice'".into()),
    };

    let invoice = match ldk_node::lightning_invoice::Bolt11Invoice::from_str(invoice_str) {
        Ok(inv) => inv,
        Err(e) => return err(format!("failed to parse invoice: {}", e)),
    };

    let payment_hash = invoice.payment_hash().to_string();
    let amount_msat = invoice.amount_milli_satoshis();
    let description = format!("{}", invoice.description());
    let expiry_secs = invoice.expiry_time().as_secs();

    ok(serde_json::json!({
        "payment_hash": payment_hash,
        "amount_msat": amount_msat,
        "description": description,
        "expiry_secs": expiry_secs,
    }))
}

fn handle_get_gossip_metadata(state: &AppState) -> (bool, serde_json::Value) {
    let node = state.node_manager.node();
    match node.custom_gossip() {
        Some(gossip) => {
            let all_metadata = gossip.get_all_metadata();
            let nodes: Vec<serde_json::Value> = all_metadata
                .iter()
                .map(|(node_id, entry)| {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    entry.metadata.hash(&mut hasher);
                    let content_hash = format!("{:016x}", hasher.finish());

                    let metadata = String::from_utf8(entry.metadata.clone())
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .unwrap_or(serde_json::Value::Null);

                    serde_json::json!({
                        "node_id": node_id.to_string(),
                        "metadata": metadata,
                        "content_hash": content_hash,
                    })
                })
                .collect();
            ok(serde_json::json!({ "nodes": nodes }))
        }
        None => ok(serde_json::json!({ "nodes": [] })),
    }
}

fn handle_set_gossip_metadata(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let node = state.node_manager.node();
    match node.custom_gossip() {
        Some(gossip) => {
            // Accept metadata as JSON value, serialize to bytes
            let metadata = match payload.get("metadata") {
                Some(m) => m,
                None => return err("missing required field 'metadata'".into()),
            };

            let metadata_bytes = match serde_json::to_vec(metadata) {
                Ok(b) => b,
                Err(e) => return err(format!("failed to serialize metadata: {}", e)),
            };

            gossip.set_our_metadata(metadata_bytes);
            ok(serde_json::json!({ "status": "updated" }))
        }
        None => err("custom gossip not available".into()),
    }
}

fn handle_verify_signature(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let message_b64 = match payload.get("message").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'message'".into()),
    };
    let signature_hex = match payload.get("signature").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'signature'".into()),
    };
    let public_key_hex = match payload.get("public_key").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'public_key'".into()),
    };

    // Decode message from base64
    use base64::Engine;
    let message_bytes = match base64::engine::general_purpose::STANDARD.decode(message_b64) {
        Ok(b) => b,
        Err(e) => return err(format!("invalid base64 message: {}", e)),
    };

    // Decode signature from hex
    let signature_str = signature_hex.to_string();

    // Parse public key
    let public_key: ldk_node::bitcoin::secp256k1::PublicKey = match public_key_hex.parse() {
        Ok(pk) => pk,
        Err(e) => return err(format!("invalid public_key: {}", e)),
    };

    let node = state.node_manager.node();
    let is_valid = node.verify_signature(&message_bytes, &signature_str, &public_key);

    ok(serde_json::json!({ "valid": is_valid }))
}

fn handle_refresh_peers(state: &AppState) -> (bool, serde_json::Value) {
    let node = state.node_manager.node();
    let peers = node.list_peers();
    let peer_count = peers.len();

    // Disconnect all peers
    for peer in &peers {
        let _ = node.disconnect(peer.node_id);
    }

    // Reconnect all peers
    let mut reconnected = 0u32;
    for peer in &peers {
        if let Ok(()) = node.connect(peer.node_id, peer.address.clone(), true) { reconnected += 1 }
    }

    ok(serde_json::json!({
        "total_peers": peer_count,
        "reconnected": reconnected,
    }))
}

// ============================================================================
// core.l402.* handlers
// ============================================================================

fn handle_l402_create_challenge(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let path = match payload.get("path").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'path'".into()),
    };
    let method = match payload.get("method").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'method'".into()),
    };

    // If amount_sats is provided, use custom challenge (dynamic pricing)
    if let Some(amount_sats) = payload.get("amount_sats").and_then(|v| v.as_i64()) {
        let description = payload
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("L402 payment");
        match state.l402_service.create_custom_challenge(path, method, amount_sats, description) {
            Ok(response) => return ok(response),
            Err(e) => return err(format!("failed to create custom L402 challenge: {}", e)),
        }
    }

    // Standard fee-table lookup
    match state.l402_service.create_challenge(path, method) {
        Ok(response) => ok(response),
        Err(e) => err(format!("failed to create L402 challenge: {}", e)),
    }
}

fn handle_l402_verify_auth(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let token = match payload.get("token").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'token'".into()),
    };
    let path = match payload.get("path").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'path'".into()),
    };
    // Method is optional — the L402 middleware only sends token+path.
    // Default to "*" which the macaroon service should accept for any method.
    let method = payload.get("method").and_then(|v| v.as_str()).unwrap_or("*");

    match state.l402_service.verify_auth(token, path, method) {
        Ok(result) => ok(result),
        Err(e) => ok(serde_json::json!({
            "valid": false,
            "payment_hash": null,
            "error": e,
        })),
    }
}

fn handle_l402_get_invoice_by_hash(state: &AppState, payload: &serde_json::Value) -> (bool, serde_json::Value) {
    let payment_hash = match payload.get("payment_hash").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return err("missing required field 'payment_hash'".into()),
    };

    match state.l402_repo.get_invoice_by_payment_hash(payment_hash) {
        Ok(Some(invoice)) => ok(serde_json::json!({
            "invoice": {
                "id": invoice.id,
                "payment_hash": invoice.payment_hash,
                "preimage": invoice.preimage,
                "bolt11": invoice.bolt11,
                "endpoint_path": invoice.endpoint_path,
                "amount_sats": invoice.amount_sats,
                "status": invoice.status,
                "paid_at": invoice.paid_at,
            }
        })),
        Ok(None) => err("invoice not found".into()),
        Err(e) => err(format!("database error: {}", e)),
    }
}

fn handle_l402_cleanup_expired(state: &AppState) -> (bool, serde_json::Value) {
    match state.l402_repo.mark_expired_invoices() {
        Ok(count) => ok(serde_json::json!({ "cleaned_count": count })),
        Err(e) => err(format!("cleanup failed: {}", e)),
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn ok(payload: serde_json::Value) -> (bool, serde_json::Value) {
    (true, payload)
}

fn err(message: String) -> (bool, serde_json::Value) {
    (false, serde_json::json!({ "error": message }))
}
