//! History ของบรรทัดคำสั่ง — เก็บในหน่วยความจำ + บันทึกต่อท้ายไฟล์เพื่อใช้ข้าม session

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

pub struct History {
    entries: Vec<String>,
    /// ตำแหน่งที่กำลังเลื่อนดู; `== entries.len()` หมายถึงไม่ได้เลื่อนดู (บรรทัดปัจจุบัน)
    cursor: usize,
    /// บรรทัดที่พิมพ์ค้างไว้ก่อนกด ↑ (คืนให้เมื่อกด ↓ จนสุด)
    draft: String,
    path: Option<PathBuf>,
    max_entries: usize,
}

impl History {
    #[allow(dead_code)]
    pub fn in_memory() -> Self {
        Self { entries: vec![], cursor: 0, draft: String::new(), path: None, max_entries: 1000 }
    }

    /// `~/.decision-dev_history` (HOME บน Unix, USERPROFILE บน Windows) ถ้าไม่พบใช้ไดเรกทอรีปัจจุบัน
    pub fn default_path() -> PathBuf {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        match home {
            Some(h) => PathBuf::from(h).join(".decision-dev_history"),
            None => PathBuf::from(".decision-dev_history"),
        }
    }

    pub fn load(path: PathBuf, max_entries: usize) -> Self {
        let mut entries: Vec<String> = fs::read_to_string(&path)
            .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
            .unwrap_or_default();
        if entries.len() > max_entries {
            let cut = entries.len() - max_entries;
            entries.drain(..cut);
            let _ = fs::write(&path, entries.join("\n") + "\n"); // ตัดไฟล์ให้เหลือ max_entries
        }
        let cursor = entries.len();
        Self { entries, cursor, draft: String::new(), path: Some(path), max_entries }
    }

    /// เรียกเมื่อกด Enter — ข้ามบรรทัดว่างและบรรทัดที่ซ้ำกับล่าสุด
    pub fn push(&mut self, line: &str) {
        let line = line.trim();
        if !line.is_empty() && self.entries.last().map(String::as_str) != Some(line) {
            self.entries.push(line.to_string());
            if self.entries.len() > self.max_entries {
                self.entries.remove(0);
            }
            if let Some(p) = &self.path {
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
                    let _ = writeln!(f, "{line}");
                }
            }
        }
        self.reset_browse();
    }

    /// ↑
    pub fn prev(&mut self, current: &str) -> Option<&str> {
        if self.cursor == 0 {
            return None;
        }
        if self.cursor == self.entries.len() {
            self.draft = current.to_string();
        }
        self.cursor -= 1;
        Some(&self.entries[self.cursor])
    }

    /// ↓
    pub fn next(&mut self) -> Option<&str> {
        if self.cursor >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        if self.cursor == self.entries.len() {
            Some(&self.draft)
        } else {
            Some(&self.entries[self.cursor])
        }
    }

    pub fn reset_browse(&mut self) {
        self.cursor = self.entries.len();
        self.draft.clear();
    }
}
