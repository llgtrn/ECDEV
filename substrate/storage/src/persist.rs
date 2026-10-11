//! The durable log: an append-only file of committed transactions with a checksum per record.
//!
//! * A commit appends one record and `sync_data`s it before it returns, so a committed
//!   transaction survives a crash and a torn record at the end is ignored (and cut off by the next
//!   writer).
//! * An exclusive file lock fences writers across processes; readers take a shared lock while they
//!   catch up with the records appended since they last looked.
//! * Compaction rewrites the live data into a fresh file and renames it over the old one; the
//!   file header carries a generation, and a handle that notices a different file reloads.

use crate::{
    db::Database,
    error::{Error, ErrorKind, Result},
    value::Value,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAGIC: &[u8; 8] = b"ECDEVDB1";
const HEADER: u64 = 16;
const RECORD_MARK: u8 = 0xEC;

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Put {
        table: String,
        rowid: i64,
        row: Vec<Value>,
    },
    Del {
        table: String,
        rowid: i64,
    },
    Ddl {
        sql: String,
    },
    UserVersion(i64),
    Sequence {
        table: String,
        value: i64,
    },
}

// ---- encoding ------------------------------------------------------------------------------------

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_zigzag(out: &mut Vec<u8>, v: i64) {
    put_varint(out, ((v << 1) ^ (v >> 63)) as u64);
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

fn put_value(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Null => out.push(0),
        Value::Integer(i) => {
            out.push(1);
            put_zigzag(out, *i);
        }
        Value::Real(f) => {
            out.push(2);
            out.extend_from_slice(&f.to_bits().to_le_bytes());
        }
        Value::Text(s) => {
            out.push(3);
            put_bytes(out, s.as_bytes());
        }
        Value::Blob(b) => {
            out.push(4);
            put_bytes(out, b);
        }
    }
}

pub fn encode_ops(ops: &[Op]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, ops.len() as u64);
    for op in ops {
        match op {
            Op::Put { table, rowid, row } => {
                out.push(1);
                put_bytes(&mut out, table.as_bytes());
                put_zigzag(&mut out, *rowid);
                put_varint(&mut out, row.len() as u64);
                for v in row {
                    put_value(&mut out, v);
                }
            }
            Op::Del { table, rowid } => {
                out.push(2);
                put_bytes(&mut out, table.as_bytes());
                put_zigzag(&mut out, *rowid);
            }
            Op::Ddl { sql } => {
                out.push(3);
                put_bytes(&mut out, sql.as_bytes());
            }
            Op::UserVersion(v) => {
                out.push(4);
                put_zigzag(&mut out, *v);
            }
            Op::Sequence { table, value } => {
                out.push(5);
                put_bytes(&mut out, table.as_bytes());
                put_zigzag(&mut out, *value);
            }
        }
    }
    out
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let v = *self.b.get(self.i).ok_or_else(corrupt)?;
        self.i += 1;
        Ok(v)
    }
    fn varint(&mut self) -> Result<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let b = self.byte()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
            if shift > 63 {
                return Err(corrupt());
            }
        }
    }
    fn zigzag(&mut self) -> Result<i64> {
        let v = self.varint()?;
        Ok(((v >> 1) as i64) ^ -((v & 1) as i64))
    }
    fn bytes(&mut self) -> Result<&[u8]> {
        let n = self.varint()? as usize;
        let end = self
            .i
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or_else(corrupt)?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn string(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| corrupt())
    }
    fn value(&mut self) -> Result<Value> {
        Ok(match self.byte()? {
            0 => Value::Null,
            1 => Value::Integer(self.zigzag()?),
            2 => {
                let end = self.i + 8;
                let raw: [u8; 8] = self
                    .b
                    .get(self.i..end)
                    .ok_or_else(corrupt)?
                    .try_into()
                    .unwrap();
                self.i = end;
                Value::Real(f64::from_bits(u64::from_le_bytes(raw)))
            }
            3 => Value::Text(self.string()?),
            4 => Value::Blob(self.bytes()?.to_vec()),
            _ => return Err(corrupt()),
        })
    }
}

fn corrupt() -> Error {
    Error::new(ErrorKind::Corrupt, "database log is corrupt")
}

pub fn decode_ops(payload: &[u8]) -> Result<Vec<Op>> {
    let mut r = Reader { b: payload, i: 0 };
    let n = r.varint()? as usize;
    let mut ops = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        ops.push(match r.byte()? {
            1 => {
                let table = r.string()?;
                let rowid = r.zigzag()?;
                let cols = r.varint()? as usize;
                let mut row = Vec::with_capacity(cols.min(1 << 12));
                for _ in 0..cols {
                    row.push(r.value()?);
                }
                Op::Put { table, rowid, row }
            }
            2 => Op::Del {
                table: r.string()?,
                rowid: r.zigzag()?,
            },
            3 => Op::Ddl { sql: r.string()? },
            4 => Op::UserVersion(r.zigzag()?),
            5 => Op::Sequence {
                table: r.string()?,
                value: r.zigzag()?,
            },
            _ => return Err(corrupt()),
        });
    }
    if r.i != payload.len() {
        return Err(corrupt());
    }
    Ok(ops)
}

// ---- checksum ------------------------------------------------------------------------------------

fn crc32c(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    (c >> 1) ^ 0x82F6_3B78
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = table[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

// ---- the log file --------------------------------------------------------------------------------

pub struct LogFile {
    path: PathBuf,
    file: File,
    generation: u64,
    /// Offset up to which this handle's database image reflects the file.
    applied: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Shared,
    Exclusive,
}

fn header(generation: u64) -> [u8; 16] {
    let mut h = [0u8; 16];
    h[..8].copy_from_slice(MAGIC);
    h[8..].copy_from_slice(&generation.to_le_bytes());
    h
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.len() == b.len()
}

impl LogFile {
    pub fn open(path: &Path) -> Result<LogFile> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock()?;
        let len = file.metadata()?.len();
        if len == 0 {
            file.write_all(&header(1))?;
            file.sync_data()?;
        }
        file.seek(SeekFrom::Start(0))?;
        let mut h = [0u8; 16];
        file.read_exact(&mut h).map_err(|_| corrupt())?;
        if &h[..8] != MAGIC {
            file.unlock()?;
            return Err(Error::new(
                ErrorKind::Corrupt,
                "file is not an ECDEV database",
            ));
        }
        let generation = u64::from_le_bytes(h[8..].try_into().unwrap());
        file.unlock()?;
        Ok(LogFile {
            path: path.to_path_buf(),
            file,
            generation,
            applied: HEADER,
        })
    }

    pub fn lock(&self, mode: LockMode, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            let attempt = match mode {
                LockMode::Shared => self.file.try_lock_shared(),
                LockMode::Exclusive => self.file.try_lock(),
            };
            match attempt {
                Ok(()) => return Ok(()),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if start.elapsed() >= timeout {
                        return Err(Error::new(ErrorKind::Busy, "database is locked"));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(Error::io(e)),
            }
        }
    }

    pub fn unlock(&self) {
        let _ = self.file.unlock();
    }

    /// Brings `db` up to date with the file. Call with a lock held.
    pub fn refresh(
        &mut self,
        db: &mut Database,
        apply: &mut dyn FnMut(&mut Database, Op) -> Result<()>,
    ) -> Result<()> {
        // Another process may have replaced the file by compaction.
        let on_disk = std::fs::metadata(&self.path)?;
        let ours = self.file.metadata()?;
        if !same_file(&on_disk, &ours) {
            self.file = OpenOptions::new().read(true).write(true).open(&self.path)?;
            self.file.lock_shared()?;
            self.applied = HEADER;
            *db = Database::default();
        }
        let mut h = [0u8; 16];
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_exact(&mut h).map_err(|_| corrupt())?;
        let generation = u64::from_le_bytes(h[8..].try_into().unwrap());
        if generation != self.generation {
            self.generation = generation;
            self.applied = HEADER;
            *db = Database::default();
        }
        let len = self.file.metadata()?.len();
        if len <= self.applied {
            return Ok(());
        }
        let mut buf = vec![0u8; (len - self.applied) as usize];
        self.file.seek(SeekFrom::Start(self.applied))?;
        self.file.read_exact(&mut buf)?;
        let mut at = 0usize;
        while at + 5 <= buf.len() {
            if buf[at] != RECORD_MARK {
                break;
            }
            let n = u32::from_le_bytes(buf[at + 1..at + 5].try_into().unwrap()) as usize;
            let end = at + 5 + n + 4;
            if end > buf.len() {
                break;
            }
            let payload = &buf[at + 5..at + 5 + n];
            let crc = u32::from_le_bytes(buf[at + 5 + n..end].try_into().unwrap());
            if crc != crc32c(payload) {
                break;
            }
            for op in decode_ops(payload)? {
                apply(db, op)?;
            }
            at = end;
        }
        self.applied += at as u64;
        Ok(())
    }

    /// Appends one committed transaction. Call with the exclusive lock held and after `refresh`.
    pub fn append(&mut self, ops: &[Op]) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let payload = encode_ops(ops);
        let mut record = Vec::with_capacity(payload.len() + 9);
        record.push(RECORD_MARK);
        record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        record.extend_from_slice(&payload);
        record.extend_from_slice(&crc32c(&payload).to_le_bytes());
        // Anything after the last good record is a torn write: cut it off first.
        if self.file.metadata()?.len() > self.applied {
            self.file.set_len(self.applied)?;
        }
        self.file.seek(SeekFrom::Start(self.applied))?;
        self.file.write_all(&record)?;
        self.file.sync_data()?;
        self.applied += record.len() as u64;
        Ok(())
    }

    pub fn log_bytes(&self) -> u64 {
        self.applied - HEADER
    }

    /// Rewrites the file as a snapshot of `db` (exclusive lock held), replacing the old file.
    pub fn compact(&mut self, snapshot: &[Op]) -> Result<()> {
        let tmp = self.path.with_extension("compact");
        let _ = std::fs::remove_file(&tmp);
        let generation = self.generation + 1;
        let mut out = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        out.write_all(&header(generation))?;
        let mut size = HEADER;
        for chunk in snapshot.chunks(512) {
            let payload = encode_ops(chunk);
            let mut record = Vec::with_capacity(payload.len() + 9);
            record.push(RECORD_MARK);
            record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            record.extend_from_slice(&payload);
            record.extend_from_slice(&crc32c(&payload).to_le_bytes());
            out.write_all(&record)?;
            size += record.len() as u64;
        }
        out.sync_all()?;
        out.lock()?;
        std::fs::rename(&tmp, &self.path)?;
        if let Some(dir) = self.path.parent()
            && let Ok(d) = File::open(if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            })
        {
            let _ = d.sync_all();
        }
        // Readers holding the old inode notice the identity change and reload.
        self.file.unlock().ok();
        self.file = out;
        self.generation = generation;
        self.applied = size;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_round_trip() {
        let ops = vec![
            Op::Ddl {
                sql: "CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT)".into(),
            },
            Op::Put {
                table: "t".into(),
                rowid: -7,
                row: vec![
                    Value::Integer(i64::MIN),
                    Value::Real(-0.5),
                    Value::Text("é".into()),
                    Value::Blob(vec![0, 255]),
                    Value::Null,
                ],
            },
            Op::Del {
                table: "t".into(),
                rowid: 3,
            },
            Op::UserVersion(-2),
            Op::Sequence {
                table: "t".into(),
                value: 9,
            },
        ];
        let back = decode_ops(&encode_ops(&ops)).unwrap();
        assert_eq!(back, ops);
        let mut bad = encode_ops(&ops);
        bad.truncate(bad.len() - 1);
        assert!(decode_ops(&bad).is_err());
    }

    #[test]
    fn crc_matches_the_known_vector() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }
}
