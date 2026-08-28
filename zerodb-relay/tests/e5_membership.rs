//! Executable EXEMPLAR E5: datastore membership sharing and denial.

use ed25519_dalek::SigningKey;
use zerodb_core::auth::{
    AdmissionToken, DeviceCert, KnownGrant, SCOPE_READ, SCOPE_SYNC, device_pk_from_seed,
    issue_device_cert, sign_admission_token,
};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{
    MSG_AUTH, MSG_ERROR, MSG_HELLO, MSG_SUBSCRIBE, MSG_SUBSCRIBED, MSG_SYNC_REQUEST,
    MSG_SYNC_RESPONSE, peer_id_from_pk, sign_auth_for_hello,
};
use zerodb_relay::{Relay, RelaySession};

const DS_BYTES: [u8; 32] = [0x51; 32];
const GRANT_ID: [u8; 32] = [0x75; 32];
const MEMBER_ROOT: [u8; 32] = [0x11; 32];
const MEMBER_DEVICE: [u8; 32] = [0x22; 32];
const OUTSIDER_DEVICE: [u8; 32] = [0x33; 32];

fn ds() -> String {
    hex::encode(DS_BYTES)
}

fn map_get<'a>(value: &'a Cbor, key: &str) -> &'a Cbor {
    match value {
        Cbor::Map(entries) => entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("missing {key}")),
        _ => panic!("not a map"),
    }
}

fn as_u64(value: &Cbor) -> u64 {
    match value {
        Cbor::Uint(value) => *value,
        _ => panic!("not uint"),
    }
}

fn as_bytes(value: &Cbor) -> &[u8] {
    match value {
        Cbor::Bytes(value) => value,
        _ => panic!("not bytes"),
    }
}

fn encode_env(ty: u8, request_id: u32, payload: Cbor) -> Vec<u8> {
    cbor::encode(&Cbor::Map(vec![
        ("type".into(), Cbor::Uint(ty as u64)),
        ("request_id".into(), Cbor::Uint(request_id as u64)),
        ("payload".into(), payload),
    ]))
    .unwrap()
}

fn decode_env(frame: &[u8]) -> (u8, Cbor) {
    let value = cbor::decode(frame).unwrap();
    (
        as_u64(map_get(&value, "type")) as u8,
        map_get(&value, "payload").clone(),
    )
}

fn handshake(session: &mut RelaySession, seed: &[u8; 32]) {
    let key = SigningKey::from_bytes(seed);
    let public_key = key.verifying_key().to_bytes();
    let hello = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            (
                "peer_id".into(),
                Cbor::Bytes(peer_id_from_pk(&public_key).to_vec()),
            ),
            ("public_key".into(), Cbor::Bytes(public_key.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    );
    let challenge = session.handle(&hello).unwrap();
    let (_, payload) = decode_env(&challenge[0]);
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(as_bytes(map_get(&payload, "nonce")));
    let signature = sign_auth_for_hello(seed, &public_key, &[] as &[&str], &nonce);
    let auth = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(signature.to_vec()))]),
    );
    session.handle(&auth).unwrap();
}

fn cert_cbor(cert: &DeviceCert) -> Cbor {
    Cbor::Map(vec![
        ("kr".into(), Cbor::Uint(cert.kr)),
        ("device".into(), Cbor::Bytes(cert.device_pk.to_vec())),
        ("principal".into(), Cbor::Bytes(cert.principal_id.to_vec())),
        ("root_pk".into(), Cbor::Bytes(cert.root_pk.to_vec())),
        ("issued".into(), Cbor::Uint(cert.issued)),
        (
            "expiry".into(),
            cert.expiry.map(Cbor::Uint).unwrap_or(Cbor::Null),
        ),
        (
            "revoke_of".into(),
            cert.revoke_of
                .map(|value| Cbor::Bytes(value.to_vec()))
                .unwrap_or(Cbor::Null),
        ),
        ("cert_sig".into(), Cbor::Bytes(cert.cert_sig.to_vec())),
    ])
}

fn token_cbor(token: &AdmissionToken) -> Cbor {
    Cbor::Map(vec![
        ("ds".into(), Cbor::Bytes(token.ds.to_vec())),
        ("subject".into(), Cbor::Bytes(token.subject.to_vec())),
        ("grant".into(), Cbor::Bytes(token.grant.to_vec())),
        (
            "scopes".into(),
            Cbor::Array(
                token
                    .scopes
                    .iter()
                    .map(|scope| Cbor::Uint(*scope))
                    .collect(),
            ),
        ),
        ("device".into(), Cbor::Bytes(token.device.to_vec())),
        ("cert".into(), cert_cbor(&token.cert)),
        ("sig".into(), Cbor::Bytes(token.sig.to_vec())),
    ])
}

fn subscribe(session: &mut RelaySession, token: Option<&AdmissionToken>) -> u8 {
    let item = match token {
        Some(token) => Cbor::Map(vec![
            ("id".into(), Cbor::Text(ds())),
            ("token".into(), token_cbor(token)),
        ]),
        None => Cbor::Text(ds()),
    };
    let frame = encode_env(
        MSG_SUBSCRIBE,
        2,
        Cbor::Map(vec![("datastores".into(), Cbor::Array(vec![item]))]),
    );
    let response = session.handle(&frame).unwrap();
    decode_env(&response[0]).0
}

fn sync(session: &mut RelaySession) -> u8 {
    let frame = encode_env(
        MSG_SYNC_REQUEST,
        3,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(ds())),
            ("accepted_root".into(), Cbor::Bytes(vec![0; 32])),
        ]),
    );
    let response = session.handle(&frame).unwrap();
    decode_env(&response[0]).0
}

fn member_token() -> (KnownGrant, AdmissionToken) {
    let device_pk = device_pk_from_seed(&MEMBER_DEVICE);
    let cert = issue_device_cert(&MEMBER_ROOT, device_pk, 1, None).unwrap();
    let grant = KnownGrant {
        id: GRANT_ID,
        ds: DS_BYTES,
        subject: cert.principal_id,
        scopes: vec![SCOPE_READ, SCOPE_SYNC],
        expiry: None,
        revoked: false,
    };
    let token = sign_admission_token(
        &MEMBER_DEVICE,
        AdmissionToken {
            ds: DS_BYTES,
            subject: cert.principal_id,
            grant: GRANT_ID,
            scopes: vec![SCOPE_READ, SCOPE_SYNC],
            device: device_pk,
            cert,
            sig: [0; 64],
        },
    )
    .unwrap();
    (grant, token)
}

#[test]
fn exemplar_e5_membership_share_deny_and_revoke() {
    let relay = Relay::memory();
    let (grant, token) = member_token();
    relay.upsert_grant(grant).unwrap();

    // A guessed datastore id is not enough, even after transport AUTH.
    let mut outsider = relay.accept();
    handshake(&mut outsider, &OUTSIDER_DEVICE);
    assert_eq!(subscribe(&mut outsider, None), MSG_ERROR);
    assert_eq!(sync(&mut outsider), MSG_ERROR);

    // Sharing the signed capability admits the intended principal/device.
    let mut member = relay.accept();
    handshake(&mut member, &MEMBER_DEVICE);
    assert_eq!(subscribe(&mut member, Some(&token)), MSG_SUBSCRIBED);
    assert_eq!(sync(&mut member), MSG_SYNC_RESPONSE);

    // A token is not a bearer secret: replay from another transport device fails.
    let mut replayer = relay.accept();
    handshake(&mut replayer, &OUTSIDER_DEVICE);
    assert_eq!(subscribe(&mut replayer, Some(&token)), MSG_ERROR);

    // Revocation denies both new admission and the already-open member session.
    assert!(relay.revoke_grant(&ds(), &GRANT_ID).unwrap());
    assert_eq!(sync(&mut member), MSG_ERROR);
    let mut revoked = relay.accept();
    handshake(&mut revoked, &MEMBER_DEVICE);
    assert_eq!(subscribe(&mut revoked, Some(&token)), MSG_ERROR);
}

#[test]
fn membership_control_plane_survives_sqlite_reopen() {
    let path = std::env::temp_dir().join(format!(
        "zerodb-e5-membership-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&path);
    let (grant, token) = member_token();
    {
        let relay = Relay::open(&path).unwrap();
        relay.upsert_grant(grant).unwrap();
    }
    {
        let relay = Relay::open(&path).unwrap();
        let mut member = relay.accept();
        handshake(&mut member, &MEMBER_DEVICE);
        assert_eq!(subscribe(&mut member, Some(&token)), MSG_SUBSCRIBED);
        assert!(relay.revoke_grant(&ds(), &GRANT_ID).unwrap());
    }
    {
        let relay = Relay::open(&path).unwrap();
        let mut member = relay.accept();
        handshake(&mut member, &MEMBER_DEVICE);
        assert_eq!(subscribe(&mut member, Some(&token)), MSG_ERROR);
    }
    let _ = std::fs::remove_file(path);
}
