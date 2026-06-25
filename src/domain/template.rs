//! Fingerprint template domain types.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

/// A user record on the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceUser {
    pub uid: u32,
    pub user_id: String,
    pub name: String,
}

/// A decoded fingerprint template joined with its device user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerTemplate {
    pub uid: u32,
    pub fid: u8,
    pub user_id: String,
    pub name: String,
    pub template: Vec<u8>,
}

/// HRMS JSON-safe template shape.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FingerTemplatePayload {
    pub uid: u32,
    pub fid: u8,
    pub user_id: String,
    pub name: String,
    pub template_bytes: String,
}

impl FingerTemplate {
    /// Convert decoded bytes into the base64 shape expected by HRMS.
    pub fn to_payload(&self) -> FingerTemplatePayload {
        FingerTemplatePayload {
            uid: self.uid,
            fid: self.fid,
            user_id: self.user_id.clone(),
            name: self.name.clone(),
            template_bytes: STANDARD.encode(&self.template),
        }
    }
}

impl FingerTemplatePayload {
    /// Decode a HRMS payload into the internal device-template shape.
    pub fn decode(self) -> Result<FingerTemplate, String> {
        let template = STANDARD
            .decode(self.template_bytes.as_bytes())
            .map_err(|err| format!("templateBytes is invalid base64: {err}"))?;
        Ok(FingerTemplate {
            uid: self.uid,
            fid: self.fid,
            user_id: self.user_id,
            name: self.name,
            template,
        })
    }

    /// Create the matching user record for a push-user job.
    pub fn user(&self) -> DeviceUser {
        DeviceUser {
            uid: self.uid,
            user_id: self.user_id.clone(),
            name: self.name.clone(),
        }
    }
}
