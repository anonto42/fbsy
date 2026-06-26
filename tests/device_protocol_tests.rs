//! End-to-end checks of the real ZKTeco TCP connector against the mock device
//! server: attendance pull, template pull (users + fingerprints), and the
//! user/template push path.

use std::{thread, time::Duration};

use fingerbridge::{
    adapters::device_zkteco_tcp::ZktecoTcpConnector,
    application::test_server,
    config::BridgeDeviceConfig,
    domain::{DeviceUser, FingerTemplate},
    ports::device::DeviceConnector,
};

fn device_cfg(port: u16) -> BridgeDeviceConfig {
    BridgeDeviceConfig {
        device_ip: "127.0.0.1".to_string(),
        device_port: port,
        device_password: 0,
        device_timeout: 5,
        device_force_udp: false,
        device_omit_ping: true,
        device_code: "MOCK".to_string(),
        api_key: "k".to_string(),
        organization_id: 1,
        sync_interval_seconds: 300,
        clear_attendance_after_sync: false,
    }
}

fn start_mock_device(port: u16) {
    thread::spawn(move || {
        let _ = test_server::run_device(port, 3);
    });
    // Give the listener time to bind.
    thread::sleep(Duration::from_millis(400));
}

#[test]
fn real_connector_pulls_attendance_from_mock() {
    let port = 14971;
    start_mock_device(port);
    let mut client = ZktecoTcpConnector
        .connect(&device_cfg(port))
        .expect("connect");
    let records = client.pull_attendance().expect("pull attendance");
    client.disconnect();
    assert_eq!(records.len(), 3, "mock seeded 3 attendance records");
}

#[test]
fn real_connector_pulls_templates_joined_with_users() {
    let port = 14972;
    start_mock_device(port);
    let mut client = ZktecoTcpConnector
        .connect(&device_cfg(port))
        .expect("connect");
    let templates = client.get_templates().expect("get templates");
    client.disconnect();

    assert_eq!(templates.len(), 1, "mock serves one template");
    let t = &templates[0];
    assert_eq!(t.uid, 1001);
    assert_eq!(t.fid, 0);
    // Joined with the user table (uid 1001 → user_id 1001 / name MockUser).
    assert_eq!(t.user_id, "1001");
    assert_eq!(t.name, "MockUser");
    assert_eq!(t.template, vec![0xAA, 0xBB, 0xCC, 0xDD]);
}

#[test]
fn real_connector_pushes_user_template_to_mock() {
    let port = 14973;
    start_mock_device(port);
    let mut client = ZktecoTcpConnector
        .connect(&device_cfg(port))
        .expect("connect");

    let user = DeviceUser {
        uid: 2,
        user_id: "002".to_string(),
        name: "Bob".to_string(),
    };
    let finger = FingerTemplate {
        uid: 2,
        fid: 0,
        user_id: "002".to_string(),
        name: "Bob".to_string(),
        template: vec![1, 2, 3, 4, 5],
    };
    let result = client.push_user_template(&user, &finger);
    client.disconnect();
    assert!(
        result.is_ok(),
        "push should succeed against the mock: {result:?}"
    );
}
