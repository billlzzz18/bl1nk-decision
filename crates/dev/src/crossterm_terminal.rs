//! Terminal adapter บน `crossterm` (Windows / Linux / macOS) สำหรับ `terminal_cli`
//! แทน `terminal_cli_termion` ที่ใช้ได้เฉพาะ Unix

use std::collections::VecDeque;
use std::fmt::{Error as FmtError, Write as FmtWrite};
use std::io::{self, Stdout, Write};

use crossterm::cursor::MoveToColumn;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::{queue, style::Print};
use terminal_cli::{
    CharacterTerminalReader, CharacterTerminalWriter, DirectionKey, Key, TerminalError,
};

use crate::history_prompt::EditableTerminal;

pub struct CrosstermTerminal {
    out: Stdout,
    /// byte ที่เหลือของตัวอักษร UTF-8 หลายไบต์ (terminal_cli รับ Key::Character ทีละ u8)
    pending: VecDeque<u8>,
    raw: bool,
}

impl CrosstermTerminal {
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?; // error ถ้า stdin ไม่ใช่ TTY
        Ok(Self { out: io::stdout(), pending: VecDeque::new(), raw: true })
    }
}

impl Drop for CrosstermTerminal {
    fn drop(&mut self) {
        if self.raw {
            let _ = disable_raw_mode();
        }
    }
}

impl CharacterTerminalWriter for CrosstermTerminal {
    fn print(&mut self, bytes: &[u8]) {
        let _ = self.out.write_all(bytes);
        let _ = self.out.flush();
    }
}

impl FmtWrite for CrosstermTerminal {
    fn write_str(&mut self, s: &str) -> Result<(), FmtError> {
        self.print(s.as_bytes());
        Ok(())
    }
}

impl EditableTerminal for CrosstermTerminal {
    fn clear_current_line(&mut self) {
        let _ = queue!(self.out, MoveToColumn(0), Clear(ClearType::CurrentLine), Print(""));
        let _ = self.out.flush();
    }

    // ระหว่างรันคำสั่ง ออกจาก raw mode เพื่อให้ println!/tracing ของ handler ขึ้นบรรทัดถูกต้อง
    // (และ Ctrl+C ยกเลิกคำสั่งที่ค้างได้)
    fn leave_raw_mode(&mut self) {
        if self.raw {
            let _ = disable_raw_mode();
            self.raw = false;
        }
    }

    fn enter_raw_mode(&mut self) {
        if !self.raw {
            let _ = enable_raw_mode();
            self.raw = true;
        }
    }
}

impl CharacterTerminalReader for CrosstermTerminal {
    fn read(&mut self) -> Result<Key, TerminalError> {
        loop {
            if let Some(b) = self.pending.pop_front() {
                return Ok(Key::Character(b));
            }

            let ev = event::read().map_err(|_| TerminalError::Error)?;
            let Event::Key(k) = ev else { continue }; // resize / mouse / focus ข้าม

            // Windows ส่ง Press + Release → รับเฉพาะ Press/Repeat
            if k.kind == KeyEventKind::Release {
                continue;
            }

            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            match k.code {
                KeyCode::Char('c') if ctrl => return Ok(Key::Break),
                KeyCode::Char('d') if ctrl => return Ok(Key::Eot),
                KeyCode::Char(_) if ctrl => continue,
                KeyCode::Char(c) => {
                    let mut buf = [0u8; 4];
                    let bytes = c.encode_utf8(&mut buf).as_bytes();
                    self.pending.extend(&bytes[1..]);
                    return Ok(Key::Character(bytes[0]));
                }
                KeyCode::Enter => return Ok(Key::Newline),
                KeyCode::Backspace => return Ok(Key::Backspace),
                KeyCode::Tab => return Ok(Key::Tab),
                KeyCode::Up => return Ok(Key::Arrow(DirectionKey::Up)),
                KeyCode::Down => return Ok(Key::Arrow(DirectionKey::Down)),
                KeyCode::Left => return Ok(Key::Arrow(DirectionKey::Left)),
                KeyCode::Right => return Ok(Key::Arrow(DirectionKey::Right)),
                _ => continue,
            }
        }
    }
}
