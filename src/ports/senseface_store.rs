use anyhow::Result;

use crate::domain::senseface::{PendingForwardAttendance, SenseFaceAttendance, SenseFaceUser};

pub trait SenseFaceStore: Send + Sync {
    fn upsert_device(&self, serial: &str, ip: &str) -> Result<()>;

    fn save_raw_request(
        &self,
        request_hash: &str,
        serial: &str,
        table: &str,
        query: &str,
        body: &[u8],
    ) -> Result<bool>;

    fn save_attendance(&self, serial: &str, records: &[SenseFaceAttendance]) -> Result<usize>;

    fn save_attendance_record(&self, serial: &str, records: &[SenseFaceAttendance]) -> Result<usize> {
        self.save_attendance(serial, records)
    }

    fn upsert_employee(&self, serial: &str, employee: &SenseFaceUser) -> Result<()>;

    fn get_pending_forward_attendance(&self, limit: usize) -> Result<Vec<PendingForwardAttendance>>;

    fn mark_attendance_forwarded(&self, ids: &[i64]) -> Result<()>;

    fn count_missing_employees(&self, serial: &str) -> Result<usize>;
}
