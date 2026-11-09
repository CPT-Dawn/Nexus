//! The 802.1X settings shared by every Front end: wire shape and the
//! validation that runs in the TUI form, the CLI and the Daemon alike.

use nexus_ipc::{Eap, EapField, EapMethod, Op, Phase2, Secret};

fn peap() -> Eap {
    Eap {
        method: EapMethod::Peap,
        phase2: Some(Phase2::Mschapv2),
        identity: "alice@uni.example".into(),
        anonymous_identity: None,
        password: Some(Secret::new("hunter2")),
        ca_cert: None,
        domain_suffix: Some("radius.uni.example".into()),
        no_ca_validation: false,
        client_cert: None,
        private_key: None,
        private_key_password: None,
    }
}

fn fields(eap: &Eap) -> Vec<EapField> {
    eap.validate().into_iter().map(|e| e.field).collect()
}

#[test]
fn a_complete_peap_form_is_valid() {
    assert!(peap().validate().is_empty());
}

#[test]
fn identity_and_password_are_required_for_peap_and_ttls() {
    let mut eap = peap();
    eap.identity = "  ".into();
    eap.password = None;
    assert_eq!(fields(&eap), vec![EapField::Identity, EapField::Password]);
    eap.method = EapMethod::Ttls;
    eap.phase2 = Some(Phase2::Pap);
    assert_eq!(fields(&eap), vec![EapField::Identity, EapField::Password]);
}

#[test]
fn the_inner_method_must_suit_the_outer_one() {
    let mut eap = peap();
    eap.phase2 = Some(Phase2::Pap); // PAP is TTLS-only
    assert_eq!(fields(&eap), vec![EapField::Phase2]);
    eap.phase2 = None;
    assert_eq!(fields(&eap), vec![EapField::Phase2]);
    eap.method = EapMethod::Tls;
    eap.phase2 = Some(Phase2::Mschapv2); // TLS has none
    eap.password = None;
    eap.client_cert = Some("/x".into());
    eap.private_key = Some("/y".into());
    assert!(fields(&eap).contains(&EapField::Phase2));
}

#[test]
fn checking_the_server_needs_a_ca_or_a_server_name() {
    let mut eap = peap();
    eap.domain_suffix = None;
    assert_eq!(fields(&eap), vec![EapField::DomainSuffix]);
    eap.domain_suffix = Some("   ".into());
    assert_eq!(fields(&eap), vec![EapField::DomainSuffix]);
    // Skipping validation is allowed, but only said out loud.
    eap.domain_suffix = None;
    eap.no_ca_validation = true;
    assert!(eap.validate().is_empty());
    assert!(eap.skips_validation());
}

#[test]
fn a_ca_file_must_exist() {
    let mut eap = peap();
    eap.domain_suffix = None;
    eap.ca_cert = Some("/nonexistent/ca.pem".into());
    assert_eq!(fields(&eap), vec![EapField::CaCert]);
    let dir = std::env::temp_dir().join(format!("nexus-eap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ca = dir.join("ca.pem");
    std::fs::write(&ca, "pem").unwrap();
    eap.ca_cert = Some(ca.to_str().unwrap().into());
    assert!(eap.validate().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tls_needs_a_client_certificate_and_key_but_no_password() {
    let mut eap = peap();
    eap.method = EapMethod::Tls;
    eap.phase2 = None;
    eap.password = None;
    assert_eq!(
        fields(&eap),
        vec![EapField::ClientCert, EapField::PrivateKey]
    );
}

#[test]
fn the_wire_shape_round_trips_and_keeps_secrets_out_of_debug() {
    let line = r#"{"v":1,"id":3,"op":"wifi_join_eap","network":"6564","eap":{"method":"peap","phase2":"mschapv2","identity":"a@b.c","password":"hunter2","domain_suffix":"b.c"}}"#;
    let req = nexus_ipc::parse_request(line).unwrap();
    assert!(!format!("{req:?}").contains("hunter2"));
    match req.op {
        Op::WifiJoinEap { network, eap } => {
            assert_eq!(network, "6564");
            assert_eq!(eap.method, EapMethod::Peap);
            assert_eq!(eap.password.unwrap().expose(), "hunter2");
            assert!(!eap.no_ca_validation);
        }
        other => panic!("{other:?}"),
    }
}
