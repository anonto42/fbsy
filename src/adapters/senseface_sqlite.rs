use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::domain::senseface::{PendingForwardAttendance, SenseFaceAttendance, SenseFaceUser};
use crate::ports::senseface_store::SenseFaceStore;

pub struct SqliteSenseFaceStore {
    db_path: std::path::PathBuf,
}

impl SqliteSenseFaceStore {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create senseface store dir {}", parent.display()))?;
        }
        let store = Self { db_path: path };
        store.initialize()?;
        Ok(store)
    }

    fn conn(&self) -> Result<Connection> {
        let con = Connection::open(&self.db_path)
            .with_context(|| format!("open senseface db {}", self.db_path.display()))?;
        con.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=30000;",
        )
        .context("set senseface db pragmas")?;
        Ok(con)
    }

    fn initialize(&self) -> Result<()> {
        let con = self.conn()?;
        con.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS devices(
              serial_number TEXT PRIMARY KEY, ip TEXT, firmware TEXT,
              first_seen TEXT NOT NULL, last_seen TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS raw_requests(
              id INTEGER PRIMARY KEY AUTOINCREMENT, request_hash TEXT UNIQUE NOT NULL,
              serial_number TEXT NOT NULL, table_name TEXT, query_string TEXT,
              body BLOB NOT NULL, received_at TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS attendance(
              id INTEGER PRIMARY KEY AUTOINCREMENT, event_key TEXT UNIQUE NOT NULL,
              serial_number TEXT NOT NULL, employee_id TEXT NOT NULL,
              event_time TEXT NOT NULL, status TEXT, verify_mode TEXT,
              work_code TEXT, reserved TEXT, raw_line TEXT NOT NULL,
              received_at TEXT NOT NULL, forwarded_at TEXT);
            CREATE TABLE IF NOT EXISTS employees(
              serial_number TEXT NOT NULL, employee_id TEXT NOT NULL,
              name TEXT, privilege TEXT, card TEXT, raw_line TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              PRIMARY KEY(serial_number, employee_id));
            CREATE INDEX IF NOT EXISTS attendance_time_idx ON attendance(event_time);
            CREATE INDEX IF NOT EXISTS attendance_employee_idx ON attendance(employee_id);
            CREATE INDEX IF NOT EXISTS attendance_device_idx ON attendance(serial_number);
            CREATE INDEX IF NOT EXISTS attendance_forwarded_idx ON attendance(forwarded_at);
            ",
        )
        .context("initialize senseface db schema")?;
        Ok(())
    }
}

impl SenseFaceStore for SqliteSenseFaceStore {
    fn upsert_device(&self, serial: &str, ip: &str) -> Result<()> {
        let con = self.conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        con.execute(
            "INSERT INTO devices(serial_number,ip,first_seen,last_seen)
             VALUES(?,?,?,?)
             ON CONFLICT(serial_number) DO UPDATE SET
             ip=excluded.ip, last_seen=excluded.last_seen",
            rusqlite::params![serial, ip, now, now],
        )
        .context("upsert device")?;
        Ok(())
    }

    fn save_raw_request(
        &self,
        request_hash: &str,
        serial: &str,
        table: &str,
        query: &str,
        body: &[u8],
    ) -> Result<bool> {
        let con = self.conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let inserted = con
            .execute(
                "INSERT OR IGNORE INTO raw_requests
                 (request_hash,serial_number,table_name,query_string,body,received_at)
                 VALUES(?,?,?,?,?,?)",
                rusqlite::params![request_hash, serial, table, query, body, now],
            )
            .context("save raw request")?;
        Ok(inserted > 0)
    }

    fn save_attendance(&self, serial: &str, records: &[SenseFaceAttendance]) -> Result<usize> {
        if records.is_empty() {
            return Ok(0);
        }
        let con = self.conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut inserted = 0usize;
        for record in records {
            let rows = con
                .execute(
                    "INSERT OR IGNORE INTO attendance
                     (event_key,serial_number,employee_id,event_time,status,
                      verify_mode,work_code,reserved,raw_line,received_at)
                     VALUES(?,?,?,?,?,?,?,?,?,?)",
                    rusqlite::params![
                        record.event_key,
                        serial,
                        record.employee_id,
                        record.event_time,
                        record.status,
                        record.verify_mode,
                        record.work_code,
                        record.reserved,
                        record.raw_line,
                        now,
                    ],
                )
                .context("save attendance record")?;
            inserted += rows;
        }
        Ok(inserted)
    }

    fn upsert_employee(&self, serial: &str, employee: &SenseFaceUser) -> Result<()> {
        let con = self.conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        con.execute(
            "INSERT INTO employees
             (serial_number,employee_id,name,privilege,card,raw_line,updated_at)
             VALUES(?,?,?,?,?,?,?)
             ON CONFLICT(serial_number,employee_id) DO UPDATE SET
             name=excluded.name, privilege=excluded.privilege, card=excluded.card,
             raw_line=excluded.raw_line, updated_at=excluded.updated_at",
            rusqlite::params![
                serial,
                employee.employee_id,
                employee.name,
                employee.privilege,
                employee.card,
                employee.raw_line,
                now,
            ],
        )
        .context("upsert employee")?;
        Ok(())
    }

    fn get_pending_forward_attendance(
        &self,
        limit: usize,
    ) -> Result<Vec<PendingForwardAttendance>> {
        let con = self.conn()?;
        let mut stmt = con
            .prepare(
                "SELECT a.id, a.serial_number, a.employee_id, a.event_time,
                        a.status, a.verify_mode, a.work_code,
                        COALESCE(e.name, '') as employee_name
                 FROM attendance a
                 LEFT JOIN employees e ON e.serial_number = a.serial_number
                   AND e.employee_id = a.employee_id
                 WHERE a.forwarded_at IS NULL
                 ORDER BY a.id
                 LIMIT ?",
            )
            .context("prepare pending forward query")?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(PendingForwardAttendance {
                    id: row.get(0)?,
                    serial_number: row.get(1)?,
                    employee_id: row.get(2)?,
                    event_time: row.get(3)?,
                    status: row.get(4)?,
                    verify_mode: row.get(5)?,
                    work_code: row.get(6)?,
                    employee_name: row.get(7)?,
                })
            })
            .context("query pending forward attendance")?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.context("read pending forward row")?);
        }
        Ok(result)
    }

    fn mark_attendance_forwarded(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let con = self.conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        for chunk in ids.chunks(500) {
            let placeholders: Vec<String> = chunk.iter().map(|_| "?".to_string()).collect();
            let sql = format!(
                "UPDATE attendance SET forwarded_at = ? WHERE id IN ({})",
                placeholders.join(",")
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now.clone())];
            for id in chunk {
                params.push(Box::new(*id));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            con.execute(&sql, param_refs.as_slice())
                .context("mark attendance forwarded")?;
        }
        Ok(())
    }

    fn count_missing_employees(&self, serial: &str) -> Result<usize> {
        let con = self.conn()?;
        let count: i64 = con
            .query_row(
                "SELECT COUNT(*) FROM employees
                 WHERE serial_number=? AND raw_line='DISCOVERED_FROM_ATTENDANCE'",
                rusqlite::params![serial],
                |row| row.get(0),
            )
            .context("count missing employees")?;
        Ok(count as usize)
    }
}
