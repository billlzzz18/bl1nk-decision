//! ห่อ `PromptBuffer` เพื่อเพิ่ม history (↑/↓) และจัดการ UTF-8 / backspace ให้ถูกต้อง
//!
//! สาเหตุที่ต้องห่อ (ตรวจจาก source terminal_cli 0.2.0):
//! - `PromptBuffer` ข้าม `Key::Arrow(_)` (todo: line history) และ `line_buffer` เป็น private
//! - Backspace ของ `PromptBuffer` ลบทีละ byte → ทำให้ตัวอักษรไทย (UTF-8 หลายไบต์) พัง
//!
//! วิธี: ปิด echo ของ `PromptBuffer` แล้ว render เอง, เก็บ mirror ของบรรทัดไว้ที่นี่,
//! และเปลี่ยนเนื้อหาบรรทัดโดยป้อน Backspace/Character เข้า `PromptBuffer` ให้ buffer ภายในตรงกับ mirror

use terminal_cli::{
    CharacterTerminalWriter, CliExecutor, DirectionKey, Key, PromptBuffer, PromptBufferOptions,
    PromptEvent,
};

use crate::history::History;

/// ความสามารถเพิ่มเติมที่ HistoryPrompt ต้องการจาก terminal
pub trait EditableTerminal: CharacterTerminalWriter {
    fn clear_current_line(&mut self);
    fn leave_raw_mode(&mut self) {}
    fn enter_raw_mode(&mut self) {}
}

/// ดักข้อความที่ PromptBuffer พิมพ์ออก terminal (ใช้ตอน Tab autocomplete)
struct Capture<'a, T: CharacterTerminalWriter> {
    inner: &'a mut T,
    buf: Vec<u8>,
}

impl<T: CharacterTerminalWriter> CharacterTerminalWriter for Capture<'_, T> {
    fn print(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        self.inner.print(bytes);
    }
}

impl<T: CharacterTerminalWriter> std::fmt::Write for Capture<'_, T> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.print(s.as_bytes());
        Ok(())
    }
}

pub struct HistoryPrompt {
    inner: PromptBuffer,
    /// mirror ของบรรทัดที่กำลังพิมพ์
    line: String,
    /// byte ของตัวอักษร UTF-8 ที่ยังมาไม่ครบ
    utf8: Vec<u8>,
    history: History,
}

impl HistoryPrompt {
    pub fn new(prompt: &'static str, history: History) -> Self {
        let options = PromptBufferOptions {
            prompt: prompt.into(),
            echo: false, // render เอง
            ..Default::default()
        };
        Self { inner: PromptBuffer::new(options), line: String::new(), utf8: Vec::new(), history }
    }

    pub fn print_prompt<T: CharacterTerminalWriter>(&self, t: &mut T) {
        self.inner.print_prompt(t);
    }

    pub fn handle_key<T, F>(&mut self, key: Key, term: &mut T, mut call: F) -> PromptEvent
    where
        T: EditableTerminal,
        F: FnMut(&mut CliExecutor),
    {
        match key {
            Key::Character(b) => {
                self.on_byte(b, term);
                PromptEvent::Ok
            }
            Key::Backspace => {
                if let Some(ch) = self.line.pop() {
                    self.feed(Key::Backspace, ch.len_utf8(), term);
                    self.redraw(term);
                }
                PromptEvent::Ok
            }
            Key::Arrow(DirectionKey::Up) => {
                let cur = self.line.clone();
                if let Some(e) = self.history.prev(&cur).map(str::to_owned) {
                    self.replace_line(&e, term);
                }
                PromptEvent::Ok
            }
            Key::Arrow(DirectionKey::Down) => {
                if let Some(e) = self.history.next().map(str::to_owned) {
                    self.replace_line(&e, term);
                }
                PromptEvent::Ok
            }
            Key::Arrow(_) => PromptEvent::Ok, // ← → ยังไม่รองรับการแก้ไขกลางบรรทัด
            Key::Tab => {
                let mut cap = Capture { inner: term, buf: Vec::new() };
                let ev = self.inner.handle_key(Key::Tab, &mut cap, |m| call(m));
                let out = cap.buf;
                // single match → PromptBuffer พิมพ์เฉพาะส่วนที่เติม (ไม่มี newline)
                // multiple matches (Tab ครั้งที่ 2) → พิมพ์รายการ มี newline → mirror ไม่เปลี่ยน
                if !out.is_empty() && !out.contains(&b'\n') {
                    if let Ok(s) = std::str::from_utf8(&out) {
                        self.line.push_str(s);
                    }
                }
                ev
            }
            Key::Newline | Key::CarriageReturn => {
                let line = std::mem::take(&mut self.line);
                self.utf8.clear();
                self.history.push(&line);
                term.leave_raw_mode();
                let ev = self.inner.handle_key(Key::Newline, term, |m| call(m));
                term.enter_raw_mode();
                ev
            }
            Key::Break => {
                let ev = self.inner.handle_key(Key::Break, term, |m| call(m));
                self.line.clear();
                self.utf8.clear();
                self.history.reset_browse();
                ev
            }
            Key::Eot => self.inner.handle_key(Key::Eot, term, |m| call(m)),
        }
    }

    fn on_byte<T: EditableTerminal>(&mut self, b: u8, term: &mut T) {
        self.utf8.push(b);
        match std::str::from_utf8(&self.utf8) {
            Ok(s) => {
                let s = s.to_owned();
                self.utf8.clear();
                for &x in s.as_bytes() {
                    self.feed(Key::Character(x), 1, term);
                }
                self.line.push_str(&s);
                term.print_str(&s);
            }
            Err(e) if e.error_len().is_none() => {} // ตัวอักษรยังมาไม่ครบ
            Err(_) => self.utf8.clear(),            // byte เสีย
        }
    }

    /// ป้อน key เข้า PromptBuffer (echo ปิดอยู่ จึงไม่พิมพ์อะไร)
    fn feed<T: EditableTerminal>(&mut self, key: Key, times: usize, term: &mut T) {
        for _ in 0..times {
            self.inner.handle_key(key, term, |_| {});
        }
    }

    fn replace_line<T: EditableTerminal>(&mut self, new: &str, term: &mut T) {
        self.feed(Key::Backspace, self.line.len(), term); // len() = จำนวน byte
        for &b in new.as_bytes() {
            self.feed(Key::Character(b), 1, term);
        }
        self.line = new.to_string();
        self.redraw(term);
    }

    fn redraw<T: EditableTerminal>(&mut self, term: &mut T) {
        term.clear_current_line();
        self.inner.print_prompt(term);
        term.print_str(&self.line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminal_cli::CliContext;

    #[derive(Default)]
    struct Mock {
        out: Vec<u8>,
        cleared: usize,
    }
    impl CharacterTerminalWriter for Mock {
        fn print(&mut self, b: &[u8]) {
            self.out.extend_from_slice(b);
        }
    }
    impl std::fmt::Write for Mock {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            self.print(s.as_bytes());
            Ok(())
        }
    }
    impl EditableTerminal for Mock {
        fn clear_current_line(&mut self) {
            self.cleared += 1;
            self.out.clear(); // ให้ out เหลือเฉพาะสิ่งที่ redraw
        }
    }

    fn type_str(p: &mut HistoryPrompt, t: &mut Mock, s: &str, ex: &mut Vec<String>) {
        for &b in s.as_bytes() {
            p.handle_key(Key::Character(b), t, |m| record(m, ex));
        }
    }

    fn record(m: &mut CliExecutor, ex: &mut Vec<String>) {
        if let Some(ctx) = m.command("echo") {
            ex.push(ctx.get_args().trim().to_string());
        }
        let _ = m.command("help");
    }

    fn press(p: &mut HistoryPrompt, t: &mut Mock, k: Key, ex: &mut Vec<String>) {
        p.handle_key(k, t, |m| record(m, ex));
    }

    #[test]
    fn up_down_recall_and_draft() {
        let mut p = HistoryPrompt::new("> ", History::in_memory());
        let (mut t, mut ex) = (Mock::default(), vec![]);
        type_str(&mut p, &mut t, "echo one", &mut ex);
        press(&mut p, &mut t, Key::Newline, &mut ex);
        type_str(&mut p, &mut t, "echo two", &mut ex);
        press(&mut p, &mut t, Key::Newline, &mut ex);

        type_str(&mut p, &mut t, "echo dra", &mut ex); // draft
        press(&mut p, &mut t, Key::Arrow(DirectionKey::Up), &mut ex);
        assert!(String::from_utf8_lossy(&t.out).ends_with("echo two"));
        press(&mut p, &mut t, Key::Arrow(DirectionKey::Up), &mut ex);
        assert!(String::from_utf8_lossy(&t.out).ends_with("echo one"));
        press(&mut p, &mut t, Key::Arrow(DirectionKey::Down), &mut ex);
        press(&mut p, &mut t, Key::Arrow(DirectionKey::Down), &mut ex);
        assert!(String::from_utf8_lossy(&t.out).ends_with("echo dra")); // คืน draft

        press(&mut p, &mut t, Key::Arrow(DirectionKey::Up), &mut ex);
        press(&mut p, &mut t, Key::Newline, &mut ex); // รัน "echo two" ซ้ำ
        assert_eq!(ex, vec!["one", "two", "two"]);
    }

    #[test]
    fn backspace_thai_removes_whole_char() {
        let mut p = HistoryPrompt::new("> ", History::in_memory());
        let (mut t, mut ex) = (Mock::default(), vec![]);
        type_str(&mut p, &mut t, "echo กข", &mut ex);
        press(&mut p, &mut t, Key::Backspace, &mut ex);
        type_str(&mut p, &mut t, "ค", &mut ex);
        press(&mut p, &mut t, Key::Newline, &mut ex);
        assert_eq!(ex, vec!["กค"]);
    }

    #[test]
    fn tab_then_history_stays_in_sync() {
        let mut p = HistoryPrompt::new("> ", History::in_memory());
        let (mut t, mut ex) = (Mock::default(), vec![]);
        type_str(&mut p, &mut t, "ec", &mut ex);
        press(&mut p, &mut t, Key::Tab, &mut ex); // ec → echo
        type_str(&mut p, &mut t, " hi", &mut ex);
        press(&mut p, &mut t, Key::Newline, &mut ex);
        assert_eq!(ex, vec!["hi"]);

        press(&mut p, &mut t, Key::Arrow(DirectionKey::Up), &mut ex);
        press(&mut p, &mut t, Key::Newline, &mut ex);
        assert_eq!(ex, vec!["hi", "hi"]);
    }
}
