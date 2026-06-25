use fingerbridge::domain::{FingerTemplate, FingerTemplatePayload};

#[test]
fn finger_template_payload_encodes_bytes_as_base64() {
    let template = FingerTemplate {
        uid: 1,
        fid: 0,
        user_id: "001".to_string(),
        name: "Alice".to_string(),
        template: vec![1, 2, 3],
    };

    let payload = template.to_payload();

    assert_eq!(payload.uid, 1);
    assert_eq!(payload.fid, 0);
    assert_eq!(payload.user_id, "001");
    assert_eq!(payload.template_bytes, "AQID");
}

#[test]
fn finger_template_payload_decodes_base64_bytes() {
    let payload = FingerTemplatePayload {
        uid: 1,
        fid: 0,
        user_id: "001".to_string(),
        name: "Alice".to_string(),
        template_bytes: "AQID".to_string(),
    };

    let decoded = payload.decode().expect("decode template");

    assert_eq!(decoded.template, vec![1, 2, 3]);
}

#[test]
fn invalid_template_base64_is_rejected() {
    let payload = FingerTemplatePayload {
        uid: 1,
        fid: 0,
        user_id: "001".to_string(),
        name: "Alice".to_string(),
        template_bytes: "not valid base64".to_string(),
    };

    assert!(payload.decode().is_err());
}
